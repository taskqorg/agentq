use std::{env, fs, path::Path};

fn registry(directory: &Path, output: &Path) {
    let mut files: Vec<_> = fs::read_dir(directory).expect("registry directory").map(|e| e.expect("registry entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs") && p.file_stem().is_some_and(|s| s != "mod")).collect();
    files.sort();
    let mut source = String::new();
    let mut entries = Vec::new();
    for file in files {
        let name = file.file_stem().and_then(|s| s.to_str()).expect("registry module name");
        assert!(!name.is_empty() && name.bytes().enumerate().all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || i > 0 && b.is_ascii_digit()), "invalid registry module: {name}");
        source += &format!("#[path = {:?}] mod {name};\n", file.canonicalize().expect("registry module path"));
        entries.push(format!("&{name}::ENTRY"));
    }
    assert!(!entries.is_empty(), "registry has no modules");
    source += &format!("pub(super) const ENTRIES: [&Entry; {}] = [{}];\n", entries.len(), entries.join(","));
    fs::write(output, source).expect("generated registry");
}

fn decoder_fingerprint(output: &Path) {
    fn collect(path: &Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(path).expect("decoder source directory") {
            let path=entry.expect("decoder source entry").path();
            if path.is_dir(){collect(&path,files);}else if path.extension().is_some_and(|e|e=="rs"){files.push(path);}
        }
    }
    // All runtime Rust sources include every registered parser and shared normalization dependency.
    let mut files=Vec::new();collect(Path::new("src"),&mut files);
    files.extend([Path::new("Cargo.toml").to_path_buf(),Path::new("../Cargo.lock").to_path_buf(),Path::new("build.rs").to_path_buf()]);
    files.sort();
    let mut hash=ring::digest::Context::new(&ring::digest::SHA256);
    hash.update(b"agentq-history-decoder/1\0");
    for path in files {
        println!("cargo:rerun-if-changed={}",path.display());
        let label=path.to_str().expect("decoder relative path").replace('\\',"/");
        let bytes=fs::read(&path).expect("decoder source bytes");
        hash.update(&(label.len() as u64).to_be_bytes());hash.update(label.as_bytes());
        hash.update(&(bytes.len() as u64).to_be_bytes());hash.update(&bytes);
    }
    let digest=hash.finish();
    let hex:String=digest.as_ref().iter().map(|b|format!("{b:02x}")).collect();
    fs::write(output,format!("pub const DECODER: &str = \"agentq-history-decoder/1:{hex}\";\n")).expect("decoder fingerprint");
}

fn main() {
    println!("cargo:rerun-if-changed=src/agent/registry");
    let output = env::var_os("OUT_DIR").expect("OUT_DIR");
    decoder_fingerprint(&Path::new(&output).join("history-decoder.rs"));
    registry(Path::new("src/agent/registry"), &Path::new(&output).join("registry.rs"));
    registry(Path::new("src/agent/registry/examples"), &Path::new(&output).join("registry-examples.rs"));
}
