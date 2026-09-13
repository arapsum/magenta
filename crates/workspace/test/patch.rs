use super::apply_unified;

#[test]
fn applies_a_small_unified_patch() {
    let source = "one\ntwo\nthree\n";
    let patch = "--- a/example.txt\n+++ b/example.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
    assert_eq!(apply_unified(source, patch).unwrap(), "one\nTWO\nthree\n");
}

#[test]
fn rejects_stale_context() {
    let patch = "@@ -1,1 +1,1 @@\n-old\n+new\n";
    assert!(apply_unified("different\n", patch).is_err());
}
