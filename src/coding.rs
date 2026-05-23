#[cfg(test)]
mod tests {
    #[test]
    fn system_prompt_keeps_fixed_tool_projection_and_memory_readback() {
        let prompt = crate::PersonaKind::Coding.system_prompt_asset().trim();
        assert!(prompt.contains("默认会沿用用户在设置里的工具回执大小"));
        assert!(prompt.contains("固定工具投影"));
        assert!(prompt.contains("不直接暴露 `context_manage`"));
        assert!(prompt.contains("memory_check / memory_read"));
        assert!(prompt.contains("context/Coding/"));
        assert!(prompt.contains("state.json"));
        assert!(prompt.contains("只有进程需要持续交互"));
        assert!(prompt.contains("先给结论"));
    }
}
