use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing;

#[derive(Debug, Clone, Deserialize)]
pub struct CoreConfig {
    #[serde(default)]
    pub system: SystemConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub logbook: LogbookConfig,
    #[serde(default)]
    pub services: ServicesConfig,
    #[serde(default)]
    pub contracts: ContractsConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub policies: PoliciesConfig,
}

impl CoreConfig {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("config.toml");
        let mut cfg = if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("reading config file {}", path.display()))?;
            toml::from_str::<CoreConfig>(&text)
                .with_context(|| format!("parsing config file {}", path.display()))?
        } else {
            tracing::info!(
                "No config file found at {}. Using CoreConfig::default().",
                path.display()
            );
            CoreConfig::default()
        };
        cfg.resolve_paths(root);
        Ok(cfg)
    }

    fn resolve_paths(&mut self, root: &Path) {
        self.memory.cache_path = absolutize(root, &self.memory.cache_path);
        self.memory.dag_path = absolutize(root, &self.memory.dag_path);
        self.memory.archive_path = absolutize(root, &self.memory.archive_path);
        self.logbook.path = absolutize(root, &self.logbook.path);
        self.logbook.aggregate = absolutize(root, &self.logbook.aggregate);
        self.logbook.ethics_log = absolutize(root, &self.logbook.ethics_log);
        self.logbook.agent_actions = absolutize(root, &self.logbook.agent_actions);
        self.logbook.contract_violations = absolutize(root, &self.logbook.contract_violations);
        self.logbook.contracts_log = absolutize(root, &self.logbook.contracts_log);
        self.contracts.path = absolutize(root, &self.contracts.path);
        self.contracts.wasm_module_path = absolutize(root, &self.contracts.wasm_module_path);
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            system: SystemConfig::default(),
            memory: MemoryConfig::default(),
            logbook: LogbookConfig::default(),
            services: ServicesConfig::default(),
            contracts: ContractsConfig::default(),
            cache: CacheConfig::default(),
            audit: AuditConfig::default(),
            policies: PoliciesConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemConfig {
    #[serde(default = "SystemConfig::default_name")]
    pub name: String,
    #[serde(default = "SystemConfig::default_version")]
    pub version: String,
}

impl SystemConfig {
    fn default_name() -> String {
        "cogniv".to_string()
    }

    fn default_version() -> String {
        "0.1.0".to_string()
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            name: Self::default_name(),
            version: Self::default_version(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "MemoryConfig::default_cache_path")]
    pub cache_path: PathBuf,
    #[serde(default = "MemoryConfig::default_dag_path")]
    pub dag_path: PathBuf,
    #[serde(default = "MemoryConfig::default_archive_path")]
    pub archive_path: PathBuf,
}

impl MemoryConfig {
    fn default_cache_path() -> PathBuf {
        PathBuf::from("cache/memory.db")
    }

    fn default_dag_path() -> PathBuf {
        PathBuf::from("dag")
    }

    fn default_archive_path() -> PathBuf {
        PathBuf::from("archive")
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            cache_path: Self::default_cache_path(),
            dag_path: Self::default_dag_path(),
            archive_path: Self::default_archive_path(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogbookConfig {
    #[serde(default = "LogbookConfig::default_path")]
    pub path: PathBuf,
    #[serde(default = "LogbookConfig::default_aggregate")]
    pub aggregate: PathBuf,
    #[serde(default = "LogbookConfig::default_ethics_log")]
    pub ethics_log: PathBuf,
    #[serde(default = "LogbookConfig::default_agent_actions")]
    pub agent_actions: PathBuf,
    #[serde(default = "LogbookConfig::default_contract_violations")]
    pub contract_violations: PathBuf,
    #[serde(default = "LogbookConfig::default_contracts_log")]
    pub contracts_log: PathBuf,
}

impl LogbookConfig {
    fn default_path() -> PathBuf {
        PathBuf::from("logbook")
    }

    fn default_aggregate() -> PathBuf {
        PathBuf::from("logbook.jsonl")
    }

    fn default_ethics_log() -> PathBuf {
        PathBuf::from("logbook/ethics.jsonl")
    }

    fn default_agent_actions() -> PathBuf {
        PathBuf::from("logbook/actions.jsonl")
    }

    fn default_contract_violations() -> PathBuf {
        PathBuf::from("logbook/violations.jsonl")
    }

    fn default_contracts_log() -> PathBuf {
        PathBuf::from("logbook/contracts.jsonl")
    }
}

impl Default for LogbookConfig {
    fn default() -> Self {
        Self {
            path: Self::default_path(),
            aggregate: Self::default_aggregate(),
            ethics_log: Self::default_ethics_log(),
            agent_actions: Self::default_agent_actions(),
            contract_violations: Self::default_contract_violations(),
            contracts_log: Self::default_contracts_log(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServicesConfig {
    #[serde(default = "ServicesConfig::default_true")]
    pub ethos_enabled: bool,
    #[serde(default = "ServicesConfig::default_true")]
    pub librarian_enabled: bool,
    #[serde(default = "ServicesConfig::default_true")]
    pub audit_enabled: bool,
}

impl ServicesConfig {
    fn default_true() -> bool {
        true
    }
}

impl Default for ServicesConfig {
    fn default() -> Self {
        Self {
            ethos_enabled: true,
            librarian_enabled: true,
            audit_enabled: true,
        }
    }
}

// -------------------------------------------------------------------------
// Compaction config (used by services::compactor)
// -------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarizerKind {
    Heuristic,   // default
    Extractive,  // simple lead-3 / top-sentences
    Minimal,     // 1–2 key lines
    Compressive, // future LLM path
}

impl Default for SummarizerKind {
    fn default() -> Self {
        SummarizerKind::Heuristic
    }
}

impl SummarizerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SummarizerKind::Heuristic => "heuristic",
            SummarizerKind::Extractive => "extractive",
            SummarizerKind::Minimal => "minimal",
            SummarizerKind::Compressive => "compressive",
        }
    }

    #[allow(deprecated)]
    pub fn clone_or_default(&self) -> Self {
        self.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionPolicy {
    #[serde(default)]
    pub select_top_k: Option<u32>,
    #[serde(default)]
    pub prefer_rarely_accessed: bool,
    #[serde(default = "CompactionPolicy::default_archive_to_dag")]
    pub archive_to_dag: bool,
    #[serde(default)]
    pub summarizer: SummarizerKind,
    #[serde(default)]
    pub target_chars: Option<usize>, // optional target length for future use
}

impl CompactionPolicy {
    fn default_archive_to_dag() -> bool {
        true
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContractsConfig {
    #[serde(default = "ContractsConfig::default_path")]
    pub path: PathBuf,
    #[serde(default = "ContractsConfig::default_contract")]
    pub default_contract: String,
    #[serde(default = "ContractsConfig::default_accept_custom")]
    pub accept_custom: bool,
    #[serde(default = "ContractsConfig::default_require_signature")]
    pub require_signature: bool,
    #[serde(default = "ContractsConfig::default_max_rules")]
    pub max_rules: usize,
    #[serde(default = "ContractsConfig::default_max_pattern_len")]
    pub max_pattern_len: usize,
    #[serde(default = "ContractsConfig::default_max_file_kb")]
    pub max_file_kb: usize,
    #[serde(default)]
    pub allow_allow_rules: bool,
    #[serde(default)]
    pub allowed_signers: Vec<String>,
    #[serde(default = "ContractsConfig::default_wasm_enabled")]
    pub wasm_enabled: bool,
    #[serde(default = "ContractsConfig::default_wasm_module_path")]
    pub wasm_module_path: PathBuf,
    #[serde(default = "ContractsConfig::default_wasm_export")]
    pub wasm_export: String,
}

impl ContractsConfig {
    fn default_path() -> PathBuf {
        PathBuf::from("contracts")
    }

    fn default_contract() -> String {
        "nonviolence.toml".to_string()
    }
    fn default_accept_custom() -> bool {
        true
    }
    fn default_require_signature() -> bool {
        false
    }
    fn default_max_rules() -> usize {
        500
    }
    fn default_max_pattern_len() -> usize {
        256
    }
    fn default_max_file_kb() -> usize {
        256
    }
    fn default_wasm_enabled() -> bool {
        false
    }
    fn default_wasm_module_path() -> PathBuf {
        PathBuf::from("contracts/contract_eval.wasm")
    }
    fn default_wasm_export() -> String {
        "evaluate_contract".into()
    }
}

impl Default for ContractsConfig {
    fn default() -> Self {
        Self {
            path: Self::default_path(),
            default_contract: Self::default_contract(),
            accept_custom: Self::default_accept_custom(),
            require_signature: Self::default_require_signature(),
            max_rules: Self::default_max_rules(),
            max_pattern_len: Self::default_max_pattern_len(),
            max_file_kb: Self::default_max_file_kb(),
            allow_allow_rules: true,
            allowed_signers: vec![],
            wasm_enabled: Self::default_wasm_enabled(),
            wasm_module_path: Self::default_wasm_module_path(),
            wasm_export: Self::default_wasm_export(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "CacheConfig::default_max_hot_memory_mb")]
    pub max_hot_memory_mb: usize,
}

impl CacheConfig {
    fn default_max_hot_memory_mb() -> usize {
        50
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_hot_memory_mb: Self::default_max_hot_memory_mb(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditConfig {
    #[serde(default = "AuditConfig::default_retention_days")]
    pub retention_days: u32,
}

impl AuditConfig {
    fn default_retention_days() -> u32 {
        365
    }
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            retention_days: Self::default_retention_days(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoliciesConfig {
    #[serde(default = "PoliciesConfig::default_promote_hot_threshold")]
    pub promote_hot_threshold: usize,
    #[serde(default = "PoliciesConfig::default_auto_prune_duplicates")]
    pub auto_prune_duplicates: bool,
    #[serde(default = "PoliciesConfig::default_reflection_min_count")]
    pub reflection_min_count: usize,
    #[serde(default = "PoliciesConfig::default_reflection_max_keywords")]
    pub reflection_max_keywords: usize,
    #[serde(default = "PoliciesConfig::default_reflection_pool_size")]
    pub reflection_pool_size: usize,
    #[serde(default = "PoliciesConfig::default_summary_min_len")]
    pub summary_min_len: usize,
    #[serde(default = "PoliciesConfig::default_log_preview_len")]
    pub log_preview_len: usize,
}

impl PoliciesConfig {
    fn default_promote_hot_threshold() -> usize {
        5
    }

    fn default_auto_prune_duplicates() -> bool {
        true
    }

    fn default_reflection_min_count() -> usize {
        3
    }

    fn default_reflection_max_keywords() -> usize {
        3
    }

    fn default_reflection_pool_size() -> usize {
        20
    }

    fn default_summary_min_len() -> usize {
        500
    }

    fn default_log_preview_len() -> usize {
        160
    }
}

impl Default for PoliciesConfig {
    fn default() -> Self {
        Self {
            promote_hot_threshold: Self::default_promote_hot_threshold(),
            auto_prune_duplicates: Self::default_auto_prune_duplicates(),
            reflection_min_count: Self::default_reflection_min_count(),
            reflection_max_keywords: Self::default_reflection_max_keywords(),
            reflection_pool_size: Self::default_reflection_pool_size(),
            summary_min_len: Self::default_summary_min_len(),
            log_preview_len: Self::default_log_preview_len(),
        }
    }
}

fn absolutize(root: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        root.join(value)
    }
}
