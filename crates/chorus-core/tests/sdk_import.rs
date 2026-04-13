/// Verify the crate is importable as `chorus_core::`
#[test]
fn sdk_import_name() {
    let _ = chorus_core::client::Chorus::builder();
}
