/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Read initializer evidence from original mapped cache images; never execute it.
use super::{cache::CachePlan, A64Cpu};

pub(super) fn functions(cpu: &A64Cpu, plan: &CachePlan, provider: &str) -> Result<Vec<u64>, String> {
    let images = plan.images.iter().filter(|i| i.path == provider).collect::<Vec<_>>();
    if images.len() != 1 { return Err("initializer requires exactly one original provider".into()); }
    let mut readable = Vec::new();
    let mut executable = Vec::new();
    for r in &plan.regions {
        let end = r.vmaddr.checked_add(r.size).ok_or("cache range overflow")?;
        if r.init_prot & 1 != 0 { readable.push((r.vmaddr, end)); }
        if r.init_prot & 4 != 0 { executable.push((r.vmaddr, end)); }
    }
    decode(cpu, images[0].address, provider, &readable, &executable)
}
fn u32_at(b: &[u8], p: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(b.get(p..p.checked_add(4).ok_or("field overflow")?).ok_or("truncated initializer metadata")?.try_into().unwrap()))
}
fn u64_at(b: &[u8], p: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(b.get(p..p.checked_add(8).ok_or("field overflow")?).ok_or("truncated initializer metadata")?.try_into().unwrap()))
}
fn contains(ranges: &[(u64,u64)], a: u64, n: u64) -> bool {
    ranges.iter().any(|&(s,e)| a >= s && a.checked_add(n).is_some_and(|end| end <= e))
}
fn read(cpu: &A64Cpu, ranges: &[(u64,u64)], a: u64, n: usize) -> Result<Vec<u8>,String> {
    if n == 0 || n > 256*1024 || !contains(ranges,a,n as u64) { return Err("initializer metadata outside bounded readable original cache".into()); }
    let mut b = vec![0;n]; cpu.read_guest_into(a,&mut b)?; Ok(b)
}
fn decode(cpu: &A64Cpu, image: u64, provider: &str, readable: &[(u64,u64)], executable: &[(u64,u64)]) -> Result<Vec<u64>,String> {
    let h=read(cpu,readable,image,32)?;
    if u32_at(&h,0)? != 0xfeedfacf || u32_at(&h,4)? != 0x0100000c || u32_at(&h,12)? != 6 || u32_at(&h,24)? & 0x80000000 == 0 { return Err("initializer requires original cached ARM64 dylib".into()); }
    let count=u32_at(&h,16)? as usize; let size=u32_at(&h,20)? as usize;
    if count==0 || count>4096 || size>256*1024-32 { return Err("initializer load command budget exceeded".into()); }
    let b=read(cpu,readable,image,32+size)?; let mut p=32; let mut identity=false; let mut sections=Vec::new();
    for _ in 0..count {
        let cmd=u32_at(&b,p)?; let len=u32_at(&b,p+4)? as usize;
        let end=p.checked_add(len).filter(|&v|v<=b.len()).ok_or("initializer command outside header")?;
        if len<8 || len%8!=0 { return Err("invalid initializer command size".into()); }
        let c=&b[p..end];
        if cmd==0xd {
            let off=u32_at(c,8)? as usize;
            if len<24 || off<24 || off>=len || identity { return Err("invalid initializer provider identity".into()); }
            let name=&c[off..]; let nul=name.iter().position(|&v|v==0).ok_or("unterminated provider name")?;
            if &name[..nul]!=provider.as_bytes() { return Err("initializer provider install name mismatch".into()); } identity=true;
        } else if cmd==0x19 {
            if len<72 { return Err("initializer segment truncated".into()); }
            let n=u32_at(c,64)? as usize; let start=u64_at(c,24)?; let limit=start.checked_add(u64_at(c,32)?).ok_or("segment overflow")?;
            if n>128 || 72usize.checked_add(n.checked_mul(80).ok_or("section count overflow")?).is_none_or(|v|v>len) { return Err("initializer section table invalid".into()); }
            for i in 0..n {
                let s=&c[72+i*80..72+(i+1)*80]; let kind=u32_at(s,64)? & 0xff;
                if kind!=9 && kind!=0x16 { continue; }
                let a=u64_at(s,32)?; let length=u64_at(s,40)?; let stride=if kind==9 {8} else {4};
                if a<start || a.checked_add(length).is_none_or(|v|v>limit) || a%stride!=0 || length%stride!=0 || length/stride>8192 { return Err("initializer section bounds/alignment invalid".into()); }
                if length!=0 { sections.push((a,length as usize,stride as usize)); }
            }
        }
        p=end;
    }
    if !identity || p!=b.len() { return Err("initializer header identity/command coverage invalid".into()); }
    let mut result=Vec::new(); let mut ranges=Vec::new();
    for (a,n,stride) in sections {
        let end=a.checked_add(n as u64).ok_or("initializer section overflow")?;
        if ranges.iter().any(|&(s,e)|a<e&&s<end) { return Err("overlapping initializer sections".into()); } ranges.push((a,end));
        if result.len()+n/stride>8192 { return Err("initializer function count exceeded".into()); }
        let data=read(cpu,readable,a,n)?;
        for pos in (0..n).step_by(stride) {
            let target=if stride==8 {u64_at(&data,pos)?} else {image.checked_add(u32_at(&data,pos)? as u64).ok_or("initializer relative target overflow")?};
            if target%4!=0 || !contains(executable,target,4) || !cpu.mapped_permissions(target).is_some_and(|v|v&4!=0) { return Err(format!("initializer target {target:#x} is not original mapped executable memory")); }
            result.push(target);
        }
    }
    Ok(result)
}

#[cfg(test)] mod tests {
    use super::*;
    fn fixture(kind:u32)->A64Cpu {
        let mut cpu=A64Cpu::new_sparse(); cpu.map_zeroed(0x10000,4096,3).unwrap();cpu.map_zeroed(0x20000,4096,5).unwrap();
        let mut b=vec![0u8;4096];
        let put=|b:&mut[u8],p:usize,v:u32|b[p..p+4].copy_from_slice(&v.to_le_bytes());
        for(p,v)in[(0,0xfeedfacf),(4,0x0100000c),(12,6),(16,2),(20,184),(24,0x80000000),(32,0xd),(36,32),(40,24),(64,0x19),(68,152),(128,1)] {put(&mut b,p,v);}
        b[56..61].copy_from_slice(b"/lib\0");
        b[88..96].copy_from_slice(&0x10000u64.to_le_bytes()); b[96..104].copy_from_slice(&4096u64.to_le_bytes());
        b[168..176].copy_from_slice(&0x10300u64.to_le_bytes()); b[176..184].copy_from_slice(&(if kind==9 {8u64}else{4u64}).to_le_bytes());put(&mut b,200,kind);
        if kind==9 {b[768..776].copy_from_slice(&0x20020u64.to_le_bytes());}else{put(&mut b,768,0x10020);}
        cpu.write_bytes(0x10000,&b);cpu
    }
    #[test]fn image_relative_offsets_and_rebased_pointers_are_distinct(){for kind in [9,0x16] {let cpu=fixture(kind);assert_eq!(decode(&cpu,0x10000,"/lib",&[(0x10000,0x11000)],&[(0x20000,0x21000)]).unwrap(),vec![0x20020]);}}
    #[test]fn identity_and_nonexecutable_target_reject(){let cpu=fixture(0x16);assert!(decode(&cpu,0x10000,"/other",&[(0x10000,0x11000)],&[(0x20000,0x21000)]).is_err());assert!(decode(&cpu,0x10000,"/lib",&[(0x10000,0x11000)],&[]).is_err());}
}
