//! Native-presence fixture only; no fake client is executed. Existing behavior assertions use the real journal.
use std::{ffi::OsString,path::Path};

pub struct NativeClis(Option<OsString>);
impl Drop for NativeClis {
    fn drop(&mut self) {
        // The binary tests run serially (.cargo/config.toml).
        unsafe {match &self.0 {Some(path)=>std::env::set_var("PATH",path),None=>std::env::remove_var("PATH")}}
    }
}

pub fn only_path(path: &Path) -> NativeClis {
    let old=std::env::var_os("PATH");
    // Binary tests are serialized; the guard restores PATH.
    unsafe {std::env::set_var("PATH",path)}
    NativeClis(old)
}

pub fn native_clis(root: &Path) -> NativeClis {
    let bin=root.join("native-bin"); std::fs::create_dir_all(&bin).unwrap();
    for entry in agentq_runtime::agent::registry::entries().into_iter().filter(|e|!e.profile.operations.is_empty()) {
        for name in std::iter::once(entry.name).chain(entry.aliases.iter().copied()) {
            let target=bin.join(format!("{name}{}",std::env::consts::EXE_SUFFIX));
            if !target.exists() {std::fs::write(target,b"\x7fELF").unwrap();}
        }
    }
    let old=std::env::var_os("PATH");
    let path=std::env::join_paths(std::iter::once(bin).chain(old.as_deref().map(std::env::split_paths).into_iter().flatten())).unwrap();
    // The binary tests run serially and this guard restores PATH before the next test.
    unsafe {std::env::set_var("PATH",path)}
    NativeClis(old)
}
