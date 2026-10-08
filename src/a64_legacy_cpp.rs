/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/. */
//! Explicit, coherent original-cache C++ provider preparation. Extracted cache
//! artifacts and unrelated cache addresses never become ordinary dylibs here.
use super::{cache::CachePlan, cache_linker};
use std::{fs::File, io::{Read, Seek, SeekFrom}, path::Path};
const PROVIDER: &str = "/usr/lib/libstdc++.6.dylib";
const CACHE_UUID: [u8; 16] = [0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f];
const PROVIDER_UUID: [u8;16] = [0xe0,0x5b,0x7a,0x94,0x22,0x9a,0x3b,0x71,0x8a,0x28,0x61,0x0a,0x7b,0x7e,0x72,0x7d];
fn u32_at(bytes: &[u8], index: usize) -> Result<u32,String> {
    Ok(u32::from_le_bytes(bytes.get(index..index.checked_add(4).ok_or("legacy provider integer overflow")?)
        .ok_or("truncated legacy provider integer")?.try_into().unwrap()))
}
fn command_name(command: &[u8]) -> Result<&str,String> {
    let offset = usize::try_from(u32_at(command,8)?).map_err(|_|"legacy provider name offset exceeds host")?;
    if offset < 24 { return Err("legacy provider name overlaps command header".into()); }
    let tail = command.get(offset..).ok_or("legacy provider name offset outside command")?;
    let end = tail.iter().position(|&b|b==0).ok_or("unterminated legacy provider name")?;
    if end > 1024 { return Err("legacy provider name exceeds bound".into()); }
    std::str::from_utf8(&tail[..end]).map_err(|_|"legacy provider name is not UTF8".into())
}
fn validate_provider(header: &[u8], commands: &[u8]) -> Result<(),String> {
    if header.len()!=32 || u32_at(header,0)?!=0xfeedfacf || u32_at(header,4)?!=0x100000c
        || u32_at(header,12)?!=6 || u32_at(header,24)?&0x80000000==0 {
        return Err("legacy provider is not an original cached ARM64 dylib".into());
    }
    let count=u32_at(header,16)? as usize;
    if count>4096 || commands.len()>1048576 || u32_at(header,20)? as usize!=commands.len() {
        return Err("legacy provider load-command budget/size mismatch".into());
    }
    let (mut cursor,mut identity,mut uuid,mut source,mut minimum)=(0,false,false,false,false);
    let mut dependencies=Vec::new();
    for _ in 0..count {
        let kind=u32_at(commands,cursor)?;
        let size=u32_at(commands,cursor+4)? as usize;
        if size<8 || size&7!=0 {return Err("invalid legacy provider command size".into());}
        let end=cursor.checked_add(size).ok_or("legacy provider command overflow")?;
        let command=commands.get(cursor..end).ok_or("truncated legacy provider command")?;
        match kind {
            0xd => {
                if identity || command_name(command)?!=PROVIDER || u32_at(command,16)?!=(104<<16|2<<8) {
                    return Err("legacy provider install identity/version differs from 104.2".into());
                }
                identity=true;
            }
            0xc => dependencies.push(command_name(command)?.to_owned()),
            0x1b => {
                if uuid || command.len()!=24 || command[8..24]!=PROVIDER_UUID {return Err("legacy provider UUID mismatch".into());}
                uuid=true;
            }
            0x2a => {
                if source || command.len()!=16 || u64::from_le_bytes(command[8..16].try_into().unwrap())!=((104u64<<40)|(2u64<<30)) {
                    return Err("legacy provider source version differs from 104.2".into());
                }
                source=true;
            }
            0x25 => {
                if minimum || command.len()!=16 || u32_at(command,8)?!=0x000b0400 || u32_at(command,12)?!=0x000b0400 {
                    return Err("legacy provider minimum/SDK differs from iOS11.4".into());
                }
                minimum=true;
            }
            _=>{}
        }
        cursor=end;
    }
    if cursor!=commands.len() || !identity || !uuid || !source || !minimum
        || dependencies!=["/usr/lib/libSystem.B.dylib","/usr/lib/libc++abi.dylib"] {
        return Err("legacy provider original metadata/dependency contract incomplete".into());
    }
    Ok(())
}
fn read_vm(plan: &CachePlan, address: u64, length: usize) -> Result<Vec<u8>,String> {
    if length>1048576 {return Err("legacy provider read exceeds metadata budget".into());}
    let mut result=vec![0;length];
    let mut completed=0;
    while completed<length {
        let current=address.checked_add(completed as u64).ok_or("legacy provider VM overflow")?;
        let region=plan.regions.iter().find(|r| current>=r.vmaddr && current-r.vmaddr<r.size && r.init_prot&1!=0)
            .ok_or("legacy provider read outside original readable mapping")?;
        let available=region.size-(current-region.vmaddr);
        let amount=available.min((length-completed) as u64) as usize;
        let offset=region.file_offset.checked_add(current-region.vmaddr).ok_or("legacy provider file offset overflow")?;
        let mut file=File::open(&region.file).map_err(|e|e.to_string())?;
        file.seek(SeekFrom::Start(offset)).map_err(|e|e.to_string())?;
        file.read_exact(&mut result[completed..completed+amount]).map_err(|e|e.to_string())?;
        completed+=amount;
    }
    Ok(result)
}
pub(super) fn prepare(bytes:&[u8],executable_path:&str,cache_path:&Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>) -> Result<(),String> {
    let plan=CachePlan::read(cache_path)?;
    if plan.files.len()!=1 {return Err("legacy C++ preparation requires one original monolithic cache".into());}
    let mut file=File::open(&plan.files[0]).map_err(|e|e.to_string())?;
    let mut header=[0u8;104];file.read_exact(&mut header).map_err(|e|e.to_string())?;
    if header[88..104]!=CACHE_UUID {return Err("legacy C++ preparation requires verified iOS11.4.1 15G77 cache UUID".into());}
    let providers=plan.images.iter().filter(|i|i.path==PROVIDER).collect::<Vec<_>>();
    if providers.len()!=1 {return Err("original legacy C++ provider identity missing/ambiguous".into());}
    let image_header=read_vm(&plan,providers[0].address,32)?;
    let size=u32_at(&image_header,20)? as usize;
    let commands=read_vm(&plan,providers[0].address.checked_add(32).ok_or("legacy provider header overflow")?,size)?;
    validate_provider(&image_header,&commands)?;
    for dependency in ["/usr/lib/libSystem.B.dylib","/usr/lib/libc++abi.dylib"] {
        if !plan.images.iter().any(|i|i.path==dependency) {return Err(format!("original legacy C++ dependency absent: {dependency}"));}
    }
    echo!("[a64] verified original ARM64 libstdc++.6 provider104.2, source104.2, dependencies libc++abi/libSystem; coherent iOS11.4.1 cache, no mixed-cache addresses");
    drop(plan);
    let _prepared=cache_linker::prepare_with_reader_image_infos(bytes,executable_path,cache_path,reader)?;
    echo!("[a64] coherent legacy C++ application binding prepared; original runtime/34 provider constructors/exception services remain uninitialized; no execution or gameplay receipt");
    Ok(())
}
#[cfg(test)] mod tests {
    use super::*;
    fn fixture()->(Vec<u8>,Vec<u8>) {
        let mut commands=Vec::new();
        for (kind,name) in [(0xd,PROVIDER),(0xc,"/usr/lib/libSystem.B.dylib"),(0xc,"/usr/lib/libc++abi.dylib")] {
            let size=(24+name.len()+1+7)&!7;let mut command=vec![0;size];
            command[0..4].copy_from_slice(&(kind as u32).to_le_bytes());command[4..8].copy_from_slice(&(size as u32).to_le_bytes());
            command[8..12].copy_from_slice(&24u32.to_le_bytes());command[16..20].copy_from_slice(&(104u32<<16|2<<8).to_le_bytes());
            command[24..24+name.len()].copy_from_slice(name.as_bytes());commands.extend(command);
        }
        let mut uuid=vec![0;24];uuid[0..4].copy_from_slice(&0x1bu32.to_le_bytes());uuid[4..8].copy_from_slice(&24u32.to_le_bytes());uuid[8..].copy_from_slice(&PROVIDER_UUID);commands.extend(uuid);
        let mut version=vec![0;16];version[..4].copy_from_slice(&0x2au32.to_le_bytes());version[4..8].copy_from_slice(&16u32.to_le_bytes());version[8..].copy_from_slice(&((104u64<<40)|(2u64<<30)).to_le_bytes());commands.extend(version);
        let mut minimum=vec![0;16];minimum[..4].copy_from_slice(&0x25u32.to_le_bytes());minimum[4..8].copy_from_slice(&16u32.to_le_bytes());minimum[8..12].copy_from_slice(&0xb0400u32.to_le_bytes());minimum[12..].copy_from_slice(&0xb0400u32.to_le_bytes());commands.extend(minimum);
        let mut header=vec![0;32];for (offset,value) in [(0,0xfeedfacf),(4,0x100000c),(12,6),(16,6),(20,commands.len() as u32),(24,0x80000000)] {header[offset..offset+4].copy_from_slice(&value.to_le_bytes());}
        (header,commands)
    }
    #[test] fn original_identity_accepts_and_foreign_source_rejects() {
        let (header,mut commands)=fixture();validate_provider(&header,&commands).unwrap();
        commands[18]=0;assert!(validate_provider(&header,&commands).is_err());
    }
    #[test] fn extracted_or_truncated_provider_never_becomes_original_cache() {
        let (mut header,commands)=fixture();header[24..28].fill(0);
        assert!(validate_provider(&header,&commands).is_err());header[24..28].copy_from_slice(&0x80000000u32.to_le_bytes());
        for length in [0,7,commands.len()-1] {assert!(validate_provider(&header,&commands[..length]).is_err());}
    }
}
