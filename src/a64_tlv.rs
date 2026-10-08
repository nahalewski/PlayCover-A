/* This Source Code Form is subject to the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Actual mapped Mach-O TLV templates. Parsing is not TLS initialization.
use super::A64Cpu;
#[derive(Debug)] pub(super) struct Descriptor {pub slot:u64,pub offset:u64,pub thunk:u64,pub key:u64}
#[derive(Debug)] pub(super) struct Plan {
    pub header:u64,pub template:Vec<u8>,pub alignment:u64,
    pub descriptors:Vec<Descriptor>,pub initializers:Vec<u64>,
    pub preallocated_key:Option<u64>,
}
#[derive(Clone,Copy)]struct Section {address:u64,size:u64,kind:u32,align:u32}
fn u32_at(b:&[u8],p:usize)->u32 {u32::from_le_bytes(b[p..p+4].try_into().unwrap())}
fn u64_at(b:&[u8],p:usize)->u64 {u64::from_le_bytes(b[p..p+8].try_into().unwrap())}
pub(super) fn read(cpu:&A64Cpu,header:u64,slide:u64)->Result<Option<Plan>,String> {
    read_with_cache(cpu,header,slide,false)
}
pub(super) fn read_with_cache(cpu:&A64Cpu,header:u64,slide:u64,verified_original_cache:bool)->Result<Option<Plan>,String> {
    let mut h=[0u8;32];cpu.read_guest_into(header,&mut h)?;
    if u32_at(&h,0)!=0xfeedfacf || u32_at(&h,4)!=0x100000c {return Err("TLV image is not mapped ARM64 Mach-O".into());}
    let count=u32_at(&h,16) as usize;let size=u32_at(&h,20) as usize;
    if count>4096||size>1048576 {return Err("TLV load-command budget exceeded".into());}
    let mut cmds=vec![0u8;size];cpu.read_guest_into(header.checked_add(32).ok_or("TLV header overflow")?,&mut cmds)?;
    let mut cursor=0;let mut sections=Vec::new();
    for _ in 0..count {
        if cursor+8>cmds.len(){return Err("truncated TLV image command".into());}
        let command=u32_at(&cmds,cursor);let len=u32_at(&cmds,cursor+4)as usize;
        if len<8||len%4!=0||cursor.checked_add(len).is_none_or(|end|end>cmds.len()){return Err("invalid TLV image command size".into());}
        if command==0x19 {
            if len<72{return Err("truncated TLV segment command".into());}
            let segment=u64_at(&cmds,cursor+24).checked_add(slide).ok_or("TLV segment slide overflow")?;
            let end=segment.checked_add(u64_at(&cmds,cursor+32)).ok_or("TLV segment range overflow")?;
            let n=u32_at(&cmds,cursor+64)as usize;
            if n>4096||72usize.checked_add(n*80).is_none_or(|needed|needed>len){return Err("TLV sections exceed command".into());}
            for index in 0..n {
                let p=cursor+72+index*80;let kind=u32_at(&cmds,p+64)&0xff;
                if !matches!(kind,0x11|0x12|0x13|0x15){continue;}
                let address=u64_at(&cmds,p+32).checked_add(slide).ok_or("TLV section slide overflow")?;
                let bytes=u64_at(&cmds,p+40);let section_end=address.checked_add(bytes).ok_or("TLV section overflow")?;
                if address<segment||section_end>end{return Err("TLV section outside owner segment".into());}
                let align=u32_at(&cmds,p+52);
                if align>20 || address&((1u64<<align)-1)!=0 {return Err("invalid TLV section declared alignment".into());}
                if sections.len()>=4096 {return Err("TLV section total budget exceeded".into());}
                sections.push(Section{address,size:bytes,kind,align});
            }
        }
        cursor+=len;
    }
    if cursor!=cmds.len(){return Err("TLV load-command size mismatch".into());}
    if sections.is_empty(){return Ok(None);}
    let templates:Vec<_>=sections.iter().filter(|s|matches!(s.kind,0x11|0x12)&&s.size!=0).collect();
    let start=templates.iter().map(|s|s.address).min().ok_or("TLV descriptors without template")?;
    let end=templates.iter().map(|s|s.address+s.size).max().unwrap();
    let size=end-start;if size==0||size>16*1048576{return Err("TLV template allocation budget exceeded".into());}
    let mut template=vec![0u8;size as usize];let mut alignment=1u64;let mut occupied=Vec::new();
    for s in templates {
        if s.align>20{return Err("unsupported TLV template alignment".into());}
        alignment=alignment.max(1u64<<s.align);
        if occupied.iter().any(|&(a,b)|s.address<b&&a<s.address+s.size){return Err("overlapping TLV template sections".into());}
        occupied.push((s.address,s.address+s.size));
        if s.kind==0x11 {let offset=(s.address-start)as usize;cpu.read_guest_into(s.address,&mut template[offset..offset+s.size as usize])?;}
    }
    let mut descriptors=Vec::new();let mut initializers=Vec::new();let mut preallocated_key=None;
    for s in sections {
        if s.kind==0x13 {
            if s.size%24!=0||s.size/24>4096{return Err("invalid bounded LP64 TLV descriptor section".into());}
            for index in 0..s.size/24 {
                let slot=s.address+index*24;let mut raw=[0u8;24];cpu.read_guest_into(slot,&mut raw)?;
                let offset=u64_at(&raw,16);
                if offset>=size{return Err("TLV descriptor offset exceeds real template".into());}
                let key=u64_at(&raw,8);let thunk=u64_at(&raw,0);
                if key!=0 {
                    if !verified_original_cache || key>=256 {return Err(format!("TLV descriptor contains unverified preexisting key {key:#x} at {slot:#x}"));}
                    if preallocated_key.is_some_and(|old|old!=key) {return Err("inconsistent cached TLV preallocated keys".into());}
                    if thunk==0||thunk&3!=0||cpu.mapped_permissions(thunk).is_none_or(|p|p&4==0){return Err("cached TLV thunk is not physical RX".into());}
                    preallocated_key=Some(key);
                } else {cpu.validate_guest_write(slot,24)?;}
                descriptors.push(Descriptor{slot,offset,thunk,key});
                if descriptors.len()>4096{return Err("TLV descriptor total budget exceeded".into());}
            }
        } else if s.kind==0x15 {
            if s.size%8!=0||s.size/8>4096{return Err("invalid TLV initializer pointer section".into());}
            for index in 0..s.size/8 {
                let mut raw=[0u8;8];cpu.read_guest_into(s.address+index*8,&mut raw)?;let entry=u64::from_le_bytes(raw);
                if entry==0||entry&3!=0||cpu.mapped_permissions(entry).is_none_or(|p|p&4==0){return Err("TLV initializer is not physical RX".into());}
                initializers.push(entry);
                if initializers.len()>4096{return Err("TLV initializer total budget exceeded".into());}
            }
        }
    }
    if descriptors.is_empty(){return Err("TLV template has no actual variable descriptors".into());}
    if preallocated_key.is_some()&&descriptors.iter().any(|d|Some(d.key)!=preallocated_key){return Err("mixed cached assigned/unassigned TLV descriptors".into());}
    Ok(Some(Plan{header,template,alignment,descriptors,initializers,preallocated_key}))
}
