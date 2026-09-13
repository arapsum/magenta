use super::*;

#[test]
fn default_filter_is_valid() {
    EnvFilter::try_new(DEFAULT_FILTER).expect("the built-in diagnostics filter must be valid");
}

#[test]
fn log_directory_is_namespaced_for_magenta() {
    if let Ok(directory) = log_directory() {
        assert!(directory.ends_with(PathBuf::from("magenta").join("logs")));
    }
}
