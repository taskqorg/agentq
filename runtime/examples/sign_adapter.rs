//! Operator-only signing utility; prints public manifest fields, never private material.
use ring::{rand::SystemRandom, signature::{Ed25519KeyPair, KeyPair}};
use std::{fs::{self, OpenOptions}, io::Write, path::Path};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [action, key, rest @ ..] = args.as_slice() else { return Err("usage: sign_adapter keygen KEY | sign KEY ARTIFACT | seal KEY DOMAIN FILE".into()); };
    if action == "verify" && rest.len() == 1 {
        let manifest:serde_json::Value=serde_json::from_slice(&fs::read(key)?)?;
        let field=|name:&str|manifest[name].as_str().ok_or("public manifest field missing");
        agentq_runtime::agent::ProtectedHost::signed(&Path::new(&rest[0]).canonicalize()?,field("sha256")?,field("public_key")?,field("signature")?)?;
        println!("signed adapter verified");return Ok(());
    }
    if action == "keygen" && rest.is_empty() {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| "key generation failed")?;
        let mut options = OpenOptions::new(); options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(key)?; file.write_all(document.as_ref())?; file.sync_all()?;
        #[cfg(unix)] fs::File::open(Path::new(key).parent().ok_or("key parent absent")?)?.sync_all()?;
        return Ok(());
    }
    if action == "seal" && rest.len() == 2 {
        if !matches!(rest[0].as_str(),"agentq.protected-policy/1"|"agentq.retained-operation/1") {return Err("unsupported operator signing domain".into());}
        let pair=Ed25519KeyPair::from_pkcs8(&fs::read(key)?).map_err(|_|"private key invalid")?;
        let payload=fs::read_to_string(&rest[1])?;
        let mut message=rest[0].as_bytes().to_vec();message.push(0);message.extend_from_slice(payload.as_bytes());
        let signature=pair.sign(&message).as_ref().iter().map(|v|format!("{v:02x}")).collect::<String>();
        // Envelopes contain private host paths/receipts: redirect directly to protected storage, never a board/log.
        let _=serde_json::from_str::<serde_json::Value>(&payload)?;
        println!("{}",serde_json::json!({"payload":payload,"signature":signature}));return Ok(());
    }
    if action != "sign" || rest.len() != 1 { return Err("usage: sign_adapter keygen KEY | sign KEY ARTIFACT | seal KEY DOMAIN FILE".into()); }
    let pair = Ed25519KeyPair::from_pkcs8(&fs::read(key)?).map_err(|_| "private key invalid")?;
    let hash = ring::digest::digest(&ring::digest::SHA256, &fs::read(&rest[0])?);
    let hex = |b: &[u8]| b.iter().map(|v| format!("{v:02x}")).collect::<String>();
    let digest = hex(hash.as_ref());
    let signature = pair.sign(format!("agentq.protected-adapter/1\0{digest}").as_bytes());
    println!("{}", serde_json::json!({"schema":"agentq.protected-adapter/1","sha256":digest,"public_key":hex(pair.public_key().as_ref()),"signature":hex(signature.as_ref())}));
    Ok(())
}
