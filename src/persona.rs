use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersonaSettingsSurface {
    Full,
    ApiOnly,
}

impl PersonaSettingsSurface {
    pub(crate) fn allows_provider_management(self) -> bool {
        matches!(self, Self::Full)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersonaToolProfile {
    Governance,
    Focus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersonaThemeFamily {
    Matrix,
    Coding,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatThemeSpec {
    pub(crate) theme: crate::chat::Theme,
    pub(crate) cache_key: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersonaIdentitySpec {
    pub(crate) slug: &'static str,
    pub(crate) context_dir_name: &'static str,
    pub(crate) assistant_en: &'static str,
    pub(crate) assistant_zh: &'static str,
    pub(crate) tab_title: &'static str,
    pub(crate) header_badge: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersonaStorageSpec {
    pub(crate) legacy_context_rel_roots: &'static [&'static str],
    pub(crate) migrates_legacy_root_prompt_assets: bool,
    pub(crate) settings_dir_name: Option<&'static str>,
    pub(crate) legacy_settings_dir_name: Option<&'static str>,
    pub(crate) migrates_root_config_files: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersonaCapabilitySpec {
    pub(crate) provider_owner: crate::PersonaKind,
    pub(crate) settings_surface: PersonaSettingsSurface,
    pub(crate) tool_profile: PersonaToolProfile,
    pub(crate) supports_topbar: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PersonaContractSpec {
    pub(crate) identity: PersonaIdentitySpec,
    pub(crate) storage: PersonaStorageSpec,
    pub(crate) capability: PersonaCapabilitySpec,
    pub(crate) theme_family: PersonaThemeFamily,
    pub(crate) system_prompt_asset: &'static str,
}

pub struct PersonaSpec {
    pub slug: &'static str,
    pub context_dir_name: &'static str,
    pub legacy_context_rel_roots: &'static [&'static str],
    pub migrates_legacy_root_prompt_assets: bool,
    pub settings_dir_name: Option<&'static str>,
    pub legacy_settings_dir_name: Option<&'static str>,
    pub migrates_root_config_files: bool,
    pub provider_owner: crate::PersonaKind,
    pub settings_surface: PersonaSettingsSurface,
    pub tool_profile: PersonaToolProfile,
    pub theme_family: PersonaThemeFamily,
    pub system_prompt_asset: &'static str,
    pub assistant_en: &'static str,
    pub assistant_zh: &'static str,
    pub tab_title: &'static str,
    pub header_badge: &'static str,
    pub supports_topbar: bool,
}

pub struct PersonaSpecRegistry;

const MATRIX_PROMPT: &str = include_str!("../context/Matrix/prompt.txt");
const ADVISOR_PROMPT: &str = include_str!("../context/Advisor/prompt.txt");
const CODING_PROMPT: &str = include_str!("../context/Coding/prompt.txt");
const SERVER_PROMPT: &str = include_str!("../context/Server/prompt.txt");

const MATRIX_SPEC: PersonaSpec = PersonaSpec {
    slug: "matrix",
    context_dir_name: "Matrix",
    legacy_context_rel_roots: &["Matrix", "codex/matrix", "codex"],
    migrates_legacy_root_prompt_assets: true,
    settings_dir_name: Some("matrix"),
    legacy_settings_dir_name: Some("Matrix"),
    migrates_root_config_files: true,
    provider_owner: crate::PersonaKind::Matrix,
    settings_surface: PersonaSettingsSurface::Full,
    tool_profile: PersonaToolProfile::Governance,
    theme_family: PersonaThemeFamily::Matrix,
    system_prompt_asset: MATRIX_PROMPT,
    assistant_en: "Matrix",
    assistant_zh: "萤",
    tab_title: "Matrix · 萤",
    header_badge: "MATRIX",
    supports_topbar: true,
};

const CODING_SPEC: PersonaSpec = PersonaSpec {
    slug: "coding",
    context_dir_name: "Coding",
    legacy_context_rel_roots: &["Coding", "codex/coding", "codexcoding"],
    migrates_legacy_root_prompt_assets: false,
    settings_dir_name: Some("coding"),
    legacy_settings_dir_name: None,
    migrates_root_config_files: false,
    provider_owner: crate::PersonaKind::Matrix,
    settings_surface: PersonaSettingsSurface::ApiOnly,
    tool_profile: PersonaToolProfile::Focus,
    theme_family: PersonaThemeFamily::Coding,
    system_prompt_asset: CODING_PROMPT,
    assistant_en: "Coding",
    assistant_zh: "绫",
    tab_title: "Coding · 绫",
    header_badge: "CODING",
    supports_topbar: false,
};

const ADVISOR_SPEC: PersonaSpec = PersonaSpec {
    slug: "advisor",
    context_dir_name: "Advisor",
    legacy_context_rel_roots: &["Advisor", "advisor", "codex/advisor"],
    migrates_legacy_root_prompt_assets: false,
    settings_dir_name: Some("advisor"),
    legacy_settings_dir_name: None,
    migrates_root_config_files: false,
    provider_owner: crate::PersonaKind::Matrix,
    settings_surface: PersonaSettingsSurface::ApiOnly,
    tool_profile: PersonaToolProfile::Governance,
    theme_family: PersonaThemeFamily::Coding,
    system_prompt_asset: ADVISOR_PROMPT,
    assistant_en: "Advisor",
    assistant_zh: "司",
    tab_title: "司",
    header_badge: "ADVISOR",
    supports_topbar: false,
};

const SERVER_SPEC: PersonaSpec = PersonaSpec {
    slug: "server",
    context_dir_name: "Server",
    legacy_context_rel_roots: &["Server", "codex/server"],
    migrates_legacy_root_prompt_assets: false,
    settings_dir_name: Some("server"),
    legacy_settings_dir_name: None,
    migrates_root_config_files: false,
    provider_owner: crate::PersonaKind::Matrix,
    settings_surface: PersonaSettingsSurface::ApiOnly,
    tool_profile: PersonaToolProfile::Focus,
    theme_family: PersonaThemeFamily::Coding,
    system_prompt_asset: SERVER_PROMPT,
    assistant_en: "Server",
    assistant_zh: "御",
    tab_title: "Server · 御",
    header_badge: "SERVER",
    supports_topbar: false,
};

impl PersonaSpecRegistry {
    pub fn get(persona: crate::PersonaKind) -> &'static PersonaSpec {
        match persona {
            crate::PersonaKind::Matrix => &MATRIX_SPEC,
            crate::PersonaKind::Advisor => &ADVISOR_SPEC,
            crate::PersonaKind::Coding => &CODING_SPEC,
            crate::PersonaKind::Server => &SERVER_SPEC,
        }
    }
}

impl PersonaSpec {
    pub(crate) fn contract(&self) -> PersonaContractSpec {
        PersonaContractSpec {
            identity: PersonaIdentitySpec {
                slug: self.slug,
                context_dir_name: self.context_dir_name,
                assistant_en: self.assistant_en,
                assistant_zh: self.assistant_zh,
                tab_title: self.tab_title,
                header_badge: self.header_badge,
            },
            storage: PersonaStorageSpec {
                legacy_context_rel_roots: self.legacy_context_rel_roots,
                migrates_legacy_root_prompt_assets: self.migrates_legacy_root_prompt_assets,
                settings_dir_name: self.settings_dir_name,
                legacy_settings_dir_name: self.legacy_settings_dir_name,
                migrates_root_config_files: self.migrates_root_config_files,
            },
            capability: PersonaCapabilitySpec {
                provider_owner: self.provider_owner,
                settings_surface: self.settings_surface,
                tool_profile: self.tool_profile,
                supports_topbar: self.supports_topbar,
            },
            theme_family: self.theme_family,
            system_prompt_asset: self.system_prompt_asset,
        }
    }

    pub(crate) fn chat_theme_spec(&self, preset: crate::ThemePreset) -> ChatThemeSpec {
        match (preset, self.contract().theme_family) {
            (crate::ThemePreset::Rose, PersonaThemeFamily::Matrix) => ChatThemeSpec {
                theme: crate::chat::Theme {
                    fg: Color::White,
                    placeholder_fg: Color::White,
                    placeholder_bg: Color::Rgb(50, 80, 150),
                    sys_fg: Color::Rgb(255, 170, 170),
                    who_zh_fg: Color::Rgb(255, 190, 215),
                    sys_bg: Color::Rgb(85, 75, 22),
                    user_bg: Color::Rgb(40, 40, 40),
                    ai_bg: Color::Rgb(20, 70, 40),
                },
                cache_key: 1,
            },
            (crate::ThemePreset::Cyan, PersonaThemeFamily::Matrix) => ChatThemeSpec {
                theme: crate::chat::Theme {
                    fg: Color::Rgb(220, 255, 255),
                    placeholder_fg: Color::White,
                    placeholder_bg: Color::Rgb(30, 90, 120),
                    sys_fg: Color::Rgb(255, 170, 170),
                    who_zh_fg: Color::Rgb(255, 190, 215),
                    sys_bg: Color::Rgb(85, 75, 22),
                    user_bg: Color::Rgb(40, 40, 40),
                    ai_bg: Color::Rgb(20, 70, 40),
                },
                cache_key: 2,
            },
            (crate::ThemePreset::Rose, PersonaThemeFamily::Coding) => ChatThemeSpec {
                theme: crate::chat::Theme {
                    fg: Color::Rgb(232, 240, 252),
                    placeholder_fg: Color::White,
                    placeholder_bg: Color::Rgb(58, 86, 130),
                    sys_fg: Color::Rgb(180, 202, 232),
                    who_zh_fg: Color::Rgb(198, 226, 255),
                    sys_bg: Color::Rgb(42, 48, 62),
                    user_bg: Color::Rgb(36, 36, 44),
                    ai_bg: Color::Rgb(28, 44, 64),
                },
                cache_key: 3,
            },
            (crate::ThemePreset::Cyan, PersonaThemeFamily::Coding) => ChatThemeSpec {
                theme: crate::chat::Theme {
                    fg: Color::Rgb(218, 238, 255),
                    placeholder_fg: Color::White,
                    placeholder_bg: Color::Rgb(38, 92, 118),
                    sys_fg: Color::Rgb(174, 210, 255),
                    who_zh_fg: Color::Rgb(198, 228, 255),
                    sys_bg: Color::Rgb(28, 44, 58),
                    user_bg: Color::Rgb(30, 32, 36),
                    ai_bg: Color::Rgb(24, 50, 72),
                },
                cache_key: 4,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PersonaSpecRegistry;

    #[test]
    fn persona_specs_cover_all_personas() {
        for persona in crate::PersonaKind::ALL {
            let spec = PersonaSpecRegistry::get(persona);
            assert!(!spec.slug.is_empty());
            assert!(!spec.context_dir_name.is_empty());
            assert!(!spec.legacy_context_rel_roots.is_empty());
            let _ = spec.provider_owner;
            let _ = spec.settings_surface;
            let _ = spec.tool_profile;
            assert!(!spec.system_prompt_asset.trim().is_empty());
            assert!(!spec.assistant_en.is_empty());
            assert!(!spec.assistant_zh.is_empty());
            assert!(!spec.tab_title.is_empty());
            assert!(!spec.header_badge.is_empty());
        }
    }

    #[test]
    fn coding_persona_uses_api_only_settings_and_focus_tool_profile() {
        let spec = PersonaSpecRegistry::get(crate::PersonaKind::Coding);
        assert_eq!(
            spec.settings_surface,
            super::PersonaSettingsSurface::ApiOnly
        );
        assert_eq!(spec.tool_profile, super::PersonaToolProfile::Focus);
        assert!(spec.system_prompt_asset.contains("当前常驻施工工具"));
        assert!(
            spec.system_prompt_asset
                .contains("memory_read target=output")
        );
        assert!(
            spec.system_prompt_asset
                .contains("`memory/output` 实时流使用 `memory_read target=output`")
        );
        assert!(
            spec.system_prompt_asset
                .contains("ask_user` 不属于 Coding 职责")
        );
        assert!(spec.system_prompt_asset.contains("联络 Matrix"));
        assert!(!spec.system_prompt_asset.contains("使用 `ask_user`"));
    }

    #[test]
    fn advisor_persona_uses_api_only_settings_and_governance_tool_profile() {
        let spec = PersonaSpecRegistry::get(crate::PersonaKind::Advisor);
        assert_eq!(
            spec.settings_surface,
            super::PersonaSettingsSurface::ApiOnly
        );
        assert_eq!(spec.tool_profile, super::PersonaToolProfile::Governance);
        assert!(spec.system_prompt_asset.contains("Matrix · 萤"));
        assert!(spec.system_prompt_asset.contains("上下文、记忆和大回执"));
        assert!(
            spec.system_prompt_asset
                .contains("必要时直接修复 prompt/schema")
        );
        assert!(
            spec.system_prompt_asset
                .contains("context_manage target=prompt")
        );
    }

    #[test]
    fn server_persona_uses_api_only_settings_and_focus_tool_profile() {
        let spec = PersonaSpecRegistry::get(crate::PersonaKind::Server);
        assert_eq!(
            spec.settings_surface,
            super::PersonaSettingsSurface::ApiOnly
        );
        assert_eq!(spec.tool_profile, super::PersonaToolProfile::Focus);
        assert_eq!(spec.tab_title, "Server · 御");
    }

    #[test]
    fn persona_contract_exposes_identity_storage_and_capability_views() {
        let contract = PersonaSpecRegistry::get(crate::PersonaKind::Matrix).contract();
        assert_eq!(contract.identity.slug, "matrix");
        assert_eq!(contract.identity.context_dir_name, "Matrix");
        assert_eq!(contract.identity.assistant_zh, "萤");
        assert_eq!(contract.identity.tab_title, "Matrix · 萤");
        assert_eq!(contract.storage.settings_dir_name, Some("matrix"));
        assert!(contract.storage.migrates_root_config_files);
        assert_eq!(
            contract.capability.provider_owner,
            crate::PersonaKind::Matrix
        );
        assert_eq!(
            contract.capability.settings_surface,
            super::PersonaSettingsSurface::Full
        );
        assert_eq!(
            contract.capability.tool_profile,
            super::PersonaToolProfile::Governance
        );
        assert!(contract.capability.supports_topbar);
        assert_eq!(contract.theme_family, super::PersonaThemeFamily::Matrix);
        assert!(contract.system_prompt_asset.contains("Matrix"));
    }
}
