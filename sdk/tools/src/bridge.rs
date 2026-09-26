pub fn classes() -> Vec<(String, Vec<u8>)> {
    cranpose_jvm_bridge::intellij::classes(&crate::config().plugin_id)
}
