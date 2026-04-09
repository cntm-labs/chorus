/// Verify the crate is importable as `chorus::` (not `chorus_core::`)
#[test]
fn sdk_import_name() {
    let _ = chorus::client::Chorus::builder();
}
