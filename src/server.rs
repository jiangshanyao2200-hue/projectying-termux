#[cfg(test)]
mod tests {
    #[test]
    fn system_prompt_requires_server_management_and_ssh_rules() {
        let prompt = crate::PersonaKind::Server.system_prompt_asset().trim();
        assert!(prompt.contains("Server"));
        assert!(prompt.contains("服务器管理"));
        assert!(prompt.contains("SSH"));
        assert!(prompt.contains("server_manage"));
        assert!(prompt.contains("server_split"));
        assert!(prompt.contains("御A"));
        assert!(prompt.contains("pty_run"));
        assert!(prompt.contains("server_id"));
        assert!(prompt.contains("active_server_id"));
        assert!(prompt.contains("context/Server/"));
        assert!(prompt.contains("state.json"));
        assert!(prompt.contains("askpass"));
        assert!(prompt.contains("密码可在 Settings 与 server_manage 清单中明文显示"));
    }
}
