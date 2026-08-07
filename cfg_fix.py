s = open("src/config.rs").read()

# Remove the process-env-mutating precedence test. std::env::set_var/remove_var
# is unsafe in a multi-threaded test binary: mutating the global env while other
# threads call std::env::var is UB and caused flaky CI failures. The file-based
# config tests still cover from_file precedence-with-defaults fully.
old = """
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn api_secret_key_env_takes_precedence_over_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = temp_dir("env");
        let path = dir.join("config.yaml");
        std::fs::write(&path, min_yaml("from-file-secret-0123456789abcdef0123456789abcdef")).unwrap();
        unsafe {
            std::env::set_var("API_SECRET_KEY", "from-env-secret-0123456789abcdef0123456789abcdef");
        }
        let cfg = Config::from_file(&path).expect("should parse with env secret");
        assert_eq!(cfg.get_api_key().unwrap(), "from-env-secret-0123456789abcdef0123456789abcdef");
        unsafe {
            std::env::remove_var("API_SECRET_KEY");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
"""
assert old in s, "env test block not found"
s = s.replace(old, "\n}\n")

# Drop the now-unnecessary ENV_LOCK lock lines in the remaining tests.
for fn_name in [
    "fn from_file_minimal_config_applies_defaults()",
    "fn from_file_respects_explicit_fields()",
    "fn from_file_missing_file_or_secret_is_an_error()",
]:
    anchor = "    " + fn_name + " {\n        let _guard = ENV_LOCK.lock().unwrap();\n        let dir = temp_dir("
    assert anchor in s, fn_name
    s = s.replace(anchor, "    " + fn_name + " {\n        let dir = temp_dir(", 1)

open("src/config.rs", "w").write(s)
print("config.rs: env test removed, locks dropped")
