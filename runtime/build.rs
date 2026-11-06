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
    source += &format!("const ENTRIES: [&Entry; {}] = [{}];\n", entries.len(), entries.join(","));
    fs::write(output, source).expect("generated registry");
}

fn main() {
    println!("cargo:rerun-if-changed=src/agent/registry");
    let output = env::var_os("OUT_DIR").expect("OUT_DIR");
    registry(Path::new("src/agent/registry"), &Path::new(&output).join("registry.rs"));
    registry(Path::new("src/agent/registry/examples"), &Path::new(&output).join("registry-examples.rs"));
}
