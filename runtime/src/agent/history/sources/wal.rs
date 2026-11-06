//! Validate raw WAL bytes before SQLite can silently ignore a corrupt tail.
use super::*;
fn word(bytes:&[u8])->u32 {u32::from_be_bytes(bytes.try_into().expect("WAL word"))}
fn checksum(bytes:&[u8],big:bool,mut sum:[u32;2])->[u32;2] {
    for pair in bytes.chunks_exact(8) {
        let read=|b:&[u8]|if big{word(b)}else{u32::from_le_bytes(b.try_into().expect("WAL word"))};
        sum[0]=sum[0].wrapping_add(read(&pair[..4])).wrapping_add(sum[1]);
        sum[1]=sum[1].wrapping_add(read(&pair[4..])).wrapping_add(sum[0]);
    }
    sum
}
pub(super) fn committed(bytes:&[u8],index:Option<&[u8]>)->Res<usize> {
    let index=index.map(index_header).transpose()?;
    if bytes.is_empty(){if index.as_ref().is_some_and(|i|i.frames>0){return Err("history WAL committed frames missing".into());}return Ok(0);}
    if bytes.len()<32{return Err("history WAL header truncated".into());}
    let big=match word(&bytes[..4]) {0x377f0682=>false,0x377f0683=>true,_=>return Err("history WAL magic invalid".into())};
    if word(&bytes[4..8])!=3007000{return Err("history WAL version unsupported".into());}
    let page=word(&bytes[8..12]) as usize;
    if !(512..=65536).contains(&page) || !page.is_power_of_two(){return Err("history WAL page size invalid".into());}
    let mut sum=checksum(&bytes[..24],big,[0,0]);
    if sum!=[word(&bytes[24..28]),word(&bytes[28..32])]{return Err("history WAL header checksum invalid".into());}
    let size=24+page;
    let end=if let Some(index)=&index {
        if index.salt!=bytes[16..24] || index.page!=page || index.big!=big{return Err("history WAL index binding differs; snapshot unknown".into());}
        32usize.checked_add(index.frames.checked_mul(size).ok_or("history WAL frame range overflow")?).ok_or("history WAL frame range overflow")?
    }else{bytes.len()};
    let frames=bytes.get(32..end).ok_or("history WAL committed frame truncated; snapshot unknown")?;
    if frames.len()%size!=0{return Err("history WAL frame truncated; snapshot unknown".into());}
    for frame in frames.chunks_exact(size) {
        if word(&frame[..4])==0 || frame[8..16]!=bytes[16..24]{return Err("history WAL frame identity invalid; snapshot unknown".into());}
        sum=checksum(&frame[..8],big,sum);sum=checksum(&frame[24..],big,sum);
        if sum!=[word(&frame[16..20]),word(&frame[20..24])]{return Err("history WAL frame checksum invalid; snapshot unknown".into());}
    }
    if let Some(index)=index.filter(|i|i.frames>0) {
        if sum!=index.sum || word(&bytes[end-size+4..end-size+8])!=index.pages{return Err("history WAL commit differs from index; snapshot unknown".into());}
    }
    Ok(end)
}
struct Index {frames:usize,pages:u32,page:usize,big:bool,salt:Vec<u8>,sum:[u32;2]}
fn index_header(bytes:&[u8])->Res<Index> {
    let header=bytes.get(..48).ok_or("history WAL index truncated")?;
    if bytes.get(48..96)!=Some(header){return Err("history WAL index headers differ; snapshot unknown".into());}
    let big=if word(&header[..4])==3007000 {true}else if u32::from_le_bytes(header[..4].try_into().unwrap())==3007000{false}else{return Err("history WAL index version invalid".into())};
    let read=|at:usize|if big{word(&header[at..at+4])}else{u32::from_le_bytes(header[at..at+4].try_into().unwrap())};
    if header[12]!=1 || checksum(&header[..40],big,[0,0])!=[read(40),read(44)]{return Err("history WAL index checksum invalid; snapshot unknown".into());}
    let page=if big{u16::from_be_bytes(header[14..16].try_into().unwrap())}else{u16::from_le_bytes(header[14..16].try_into().unwrap())};
    Ok(Index {frames:read(16) as usize,pages:read(20),page:if page==1{65536}else{page as usize},big:header[13]!=0,salt:header[32..40].to_vec(),sum:[read(24),read(28)]})
}
