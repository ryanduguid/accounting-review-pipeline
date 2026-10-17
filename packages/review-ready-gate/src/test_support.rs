use std::{
    fs,
    path::{Path, PathBuf},
};

#[path = "../tests/support/temp.rs"]
mod temp;
pub use temp::TempDir;
pub fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}
pub fn copy_example(name: &str, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(examples().join(name)).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}
pub fn ready_pack() -> crate::models::ReadinessPack {
    crate::engine::review_pack(
        "bas",
        &examples().join("bas-ready"),
        None,
        crate::decimal::Decimal::parse("0.01").unwrap(),
        &[],
    )
    .unwrap()
}
