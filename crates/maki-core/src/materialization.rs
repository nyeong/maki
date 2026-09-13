//! Shared, IO-free contracts for source-backed generated results.
//!
//! This module deliberately does not know how code blocks or tables execute.
//! A materializer supplies trusted transform metadata, fingerprints, and source
//! edits; core associates producers with artifacts, evaluates their stored
//! provenance, and validates an all-or-none edit plan against one immutable
//! [`ProjectSnapshot`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use sha2::{Digest, Sha256};

use crate::analysis::{ProjectSnapshot, SnapshotRevision};
use crate::source::SourceSpan;

pub const PRODUCER_ID_PROPERTY: &str = "producer-id";
pub const RESULT_POLICY_PROPERTY: &str = "result-policy";
pub const MATERIALIZE_INPUT_PROPERTY: &str = "materialize-input";
pub const EFFECT_PROPERTY: &str = "effect";
pub const OUTPUT_KIND_PROPERTY: &str = "output-kind";
pub const GENERATED_FROM_PROPERTY: &str = "generated-from";
pub const GENERATED_INPUT_PROPERTY: &str = "generated-input";
pub const GENERATED_OUTPUT_PROPERTY: &str = "generated-output";
pub const GENERATED_TRANSFORM_PROPERTY: &str = "generated-transform";
pub const GENERATED_VERSION_PROPERTY: &str = "generated-version";
pub const GENERATED_EXECUTOR_PROPERTY: &str = "generated-executor";
pub const GENERATED_EXECUTOR_CONFIG_PROPERTY: &str = "generated-executor-config";
pub const GENERATED_EFFECT_PROPERTY: &str = "generated-effect";
pub const GENERATED_STATUS_PROPERTY: &str = "generated-status";
pub const GENERATED_STATUS_SUCCESS: &str = "success";

const MAX_PRODUCER_ID_LEN: usize = 128;
const MAX_OUTPUT_KIND_LEN: usize = 64;
const MAX_DECLARED_INPUT_LEN: usize = 1_024;
const MAX_TRANSFORM_COMPONENT_LEN: usize = 256;

/// A project-global, case-sensitive identity authored on a producer.
///
/// The token is intentionally independent of a document path so moving a
/// producer does not change its identity. Copying one without changing the ID
/// is detected as an ambiguous producer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProducerId(String);

impl ProducerId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProducerIdError> {
        let value = value.into();
        validate_producer_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProducerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProducerId {
    type Err = ProducerIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProducerIdError {
    Empty,
    TooLong { limit: usize },
    InvalidByte { index: usize, byte: u8 },
}

impl fmt::Display for ProducerIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("producer ID must not be empty"),
            Self::TooLong { limit } => write!(formatter, "producer ID exceeds {limit} bytes"),
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "producer ID contains invalid byte 0x{byte:02x} at offset {index}"
            ),
        }
    }
}

impl std::error::Error for ProducerIdError {}

fn validate_producer_id(value: &str) -> Result<(), ProducerIdError> {
    if value.is_empty() {
        return Err(ProducerIdError::Empty);
    }
    if value.len() > MAX_PRODUCER_ID_LEN {
        return Err(ProducerIdError::TooLong {
            limit: MAX_PRODUCER_ID_LEN,
        });
    }

    for (index, byte) in value.bytes().enumerate() {
        let valid = if index == 0 {
            byte.is_ascii_alphanumeric()
        } else {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
        };
        if !valid {
            return Err(ProducerIdError::InvalidByte { index, byte });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResultPolicy {
    Replace,
    Append,
    Frozen,
    Manual,
    Volatile,
    Cache,
}

impl ResultPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Append => "append",
            Self::Frozen => "frozen",
            Self::Manual => "manual",
            Self::Volatile => "volatile",
            Self::Cache => "cache",
        }
    }
}

impl fmt::Display for ResultPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ResultPolicy {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "replace" => Ok(Self::Replace),
            "append" => Ok(Self::Append),
            "frozen" => Ok(Self::Frozen),
            "manual" => Ok(Self::Manual),
            "volatile" => Ok(Self::Volatile),
            "cache" => Ok(Self::Cache),
            _ => Err(ParseEnumError::new("result policy", value)),
        }
    }
}

/// The minimum trusted capability class declared by a materializer/executor.
///
/// An authored `effect` property is descriptive input only. Callers must never
/// use it to downgrade this trusted value or to grant execution authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectClass {
    PureDocument,
    PureProject,
    FilesystemRead,
    Process,
    Network,
    ClockRandomSecrets,
}

impl EffectClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PureDocument => "pure-document",
            Self::PureProject => "pure-project",
            Self::FilesystemRead => "filesystem-read",
            Self::Process => "process",
            Self::Network => "network",
            Self::ClockRandomSecrets => "clock/random/secrets",
        }
    }

    pub const fn is_pure(self) -> bool {
        matches!(self, Self::PureDocument | Self::PureProject)
    }
}

impl fmt::Display for EffectClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for EffectClass {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pure-document" => Ok(Self::PureDocument),
            "pure-project" => Ok(Self::PureProject),
            "filesystem-read" => Ok(Self::FilesystemRead),
            "process" => Ok(Self::Process),
            "network" => Ok(Self::Network),
            "clock/random/secrets" => Ok(Self::ClockRandomSecrets),
            _ => Err(ParseEnumError::new("effect class", value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutputKind {
    Text,
    Maki,
    Table,
    File,
    Custom(CustomOutputKind),
}

impl OutputKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Maki => "maki",
            Self::Table => "table",
            Self::File => "file",
            Self::Custom(value) => value.as_str(),
        }
    }
}

impl fmt::Display for OutputKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for OutputKind {
    type Err = OutputKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "maki" => Ok(Self::Maki),
            "table" => Ok(Self::Table),
            "file" => Ok(Self::File),
            custom => custom.parse().map(Self::Custom),
        }
    }
}

/// A namespaced extension output kind such as `plugin/chart`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomOutputKind(String);

impl CustomOutputKind {
    pub fn new(value: impl Into<String>) -> Result<Self, OutputKindError> {
        let value = value.into();
        validate_output_kind(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CustomOutputKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CustomOutputKind {
    type Err = OutputKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputKindError {
    Empty,
    TooLong { limit: usize },
    MissingNamespace,
    InvalidByte { index: usize, byte: u8 },
}

impl fmt::Display for OutputKindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("output kind must not be empty"),
            Self::TooLong { limit } => write!(formatter, "output kind exceeds {limit} bytes"),
            Self::MissingNamespace => formatter
                .write_str("custom output kind must contain two non-empty namespace/name segments"),
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "output kind contains invalid byte 0x{byte:02x} at offset {index}"
            ),
        }
    }
}

impl std::error::Error for OutputKindError {}

fn validate_output_kind(value: &str) -> Result<(), OutputKindError> {
    if value.is_empty() {
        return Err(OutputKindError::Empty);
    }
    if value.len() > MAX_OUTPUT_KIND_LEN {
        return Err(OutputKindError::TooLong {
            limit: MAX_OUTPUT_KIND_LEN,
        });
    }
    let Some((namespace, name)) = value.split_once('/') else {
        return Err(OutputKindError::MissingNamespace);
    };
    if namespace.is_empty() || name.is_empty() || name.contains('/') {
        return Err(OutputKindError::MissingNamespace);
    }
    for (index, byte) in value.bytes().enumerate() {
        let segment_start = index == 0 || value.as_bytes()[index - 1] == b'/';
        let valid = if segment_start {
            byte.is_ascii_lowercase() || byte.is_ascii_digit()
        } else {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'/')
        };
        if !valid {
            return Err(OutputKindError::InvalidByte { index, byte });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    expected: &'static str,
    value: String,
}

impl ParseEnumError {
    fn new(expected: &'static str, value: &str) -> Self {
        Self {
            expected,
            value: value.to_string(),
        }
    }
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.expected, self.value)
    }
}

impl std::error::Error for ParseEnumError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeclaredInputKind {
    Producer,
    Block,
    Table,
    Config,
    File,
}

impl DeclaredInputKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Producer => "producer",
            Self::Block => "block",
            Self::Table => "table",
            Self::Config => "config",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DeclaredInputValue {
    Producer(ProducerId),
    Scalar(String),
}

/// A validated declared input.
///
/// Its private representation prevents programmatic callers from bypassing
/// the same scalar and producer-ID checks used by [`FromStr`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclaredInput {
    kind: DeclaredInputKind,
    value: DeclaredInputValue,
}

impl DeclaredInput {
    pub fn new(
        kind: DeclaredInputKind,
        value: impl Into<String>,
    ) -> Result<Self, DeclaredInputError> {
        let value = value.into();
        validate_declared_input_value(&value)?;
        let value = match kind {
            DeclaredInputKind::Producer => {
                DeclaredInputValue::Producer(value.parse().map_err(DeclaredInputError::Producer)?)
            }
            DeclaredInputKind::Block
            | DeclaredInputKind::Table
            | DeclaredInputKind::Config
            | DeclaredInputKind::File => DeclaredInputValue::Scalar(value),
        };
        Ok(Self { kind, value })
    }

    pub fn producer(id: ProducerId) -> Self {
        Self {
            kind: DeclaredInputKind::Producer,
            value: DeclaredInputValue::Producer(id),
        }
    }

    pub const fn kind(&self) -> DeclaredInputKind {
        self.kind
    }

    pub fn value(&self) -> &str {
        match &self.value {
            DeclaredInputValue::Producer(value) => value.as_str(),
            DeclaredInputValue::Scalar(value) => value,
        }
    }

    pub fn producer_id(&self) -> Option<&ProducerId> {
        match &self.value {
            DeclaredInputValue::Producer(id) => Some(id),
            DeclaredInputValue::Scalar(_) => None,
        }
    }
}

impl fmt::Display for DeclaredInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind().as_str(), self.value())
    }
}

impl FromStr for DeclaredInput {
    type Err = DeclaredInputError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (kind, target) = value
            .split_once(':')
            .ok_or(DeclaredInputError::MissingSeparator)?;
        let kind = match kind {
            "producer" => DeclaredInputKind::Producer,
            "block" => DeclaredInputKind::Block,
            "table" => DeclaredInputKind::Table,
            "config" => DeclaredInputKind::Config,
            "file" => DeclaredInputKind::File,
            _ => return Err(DeclaredInputError::UnknownKind(kind.to_string())),
        };
        Self::new(kind, target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredInputError {
    MissingSeparator,
    UnknownKind(String),
    EmptyValue,
    ValueTooLong { limit: usize },
    InvalidValue,
    Producer(ProducerIdError),
}

impl fmt::Display for DeclaredInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator => formatter.write_str("declared input must use kind:value"),
            Self::UnknownKind(kind) => write!(formatter, "unknown declared input kind: {kind}"),
            Self::EmptyValue => formatter.write_str("declared input value must not be empty"),
            Self::ValueTooLong { limit } => {
                write!(formatter, "declared input value exceeds {limit} bytes")
            }
            Self::InvalidValue => formatter.write_str(
                "declared input value must be trimmed and contain no control characters",
            ),
            Self::Producer(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DeclaredInputError {}

fn validate_declared_input_value(value: &str) -> Result<(), DeclaredInputError> {
    if value.is_empty() {
        return Err(DeclaredInputError::EmptyValue);
    }
    if value.len() > MAX_DECLARED_INPUT_LEN {
        return Err(DeclaredInputError::ValueTooLong {
            limit: MAX_DECLARED_INPUT_LEN,
        });
    }
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(DeclaredInputError::InvalidValue);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Fingerprint {
    type Err = FingerprintParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value
            .strip_prefix("sha256:")
            .ok_or(FingerprintParseError::InvalidPrefix)?;
        if hex.len() != 64 {
            return Err(FingerprintParseError::InvalidLength);
        }

        let mut bytes = [0; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_lower_hex(pair[0]).ok_or(FingerprintParseError::InvalidHex)?;
            let low = decode_lower_hex(pair[1]).ok_or(FingerprintParseError::InvalidHex)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintParseError {
    InvalidPrefix,
    InvalidLength,
    InvalidHex,
}

impl fmt::Display for FingerprintParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrefix => formatter.write_str("fingerprint must start with sha256:"),
            Self::InvalidLength => {
                formatter.write_str("SHA-256 fingerprint must contain 64 hex digits")
            }
            Self::InvalidHex => formatter.write_str("SHA-256 fingerprint must use lowercase hex"),
        }
    }
}

impl std::error::Error for FingerprintParseError {}

fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// A deterministic SHA-256 builder with domain separation and framed fields.
pub struct FingerprintBuilder {
    hasher: Sha256,
}

impl FingerprintBuilder {
    pub fn new(domain: &str) -> Self {
        let mut builder = Self {
            hasher: Sha256::new(),
        };
        builder
            .hasher
            .update(b"maki.materialization.fingerprint.v1");
        builder.add_framed(domain.as_bytes());
        builder
    }

    pub fn field(&mut self, name: &str, value: impl AsRef<[u8]>) -> &mut Self {
        self.add_framed(name.as_bytes());
        self.add_framed(value.as_ref());
        self
    }

    pub fn finish(self) -> Fingerprint {
        Fingerprint(self.hasher.finalize().into())
    }

    fn add_framed(&mut self, value: &[u8]) {
        self.hasher.update((value.len() as u64).to_be_bytes());
        self.hasher.update(value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransformIdentity {
    kind: String,
    version: String,
    executor: String,
    executor_config: Fingerprint,
}

impl TransformIdentity {
    pub fn new(
        kind: impl Into<String>,
        version: impl Into<String>,
        executor: impl Into<String>,
        executor_config: Fingerprint,
    ) -> Result<Self, TransformIdentityError> {
        let identity = Self {
            kind: kind.into(),
            version: version.into(),
            executor: executor.into(),
            executor_config,
        };
        validate_transform_component("kind", &identity.kind)?;
        validate_transform_component("version", &identity.version)?;
        validate_transform_component("executor", &identity.executor)?;
        Ok(identity)
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn executor(&self) -> &str {
        &self.executor
    }

    pub const fn executor_config(&self) -> Fingerprint {
        self.executor_config
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformIdentityError {
    component: &'static str,
}

impl fmt::Display for TransformIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "transform {} must be a non-empty trimmed scalar of at most {} bytes",
            self.component, MAX_TRANSFORM_COMPONENT_LEN
        )
    }
}

impl std::error::Error for TransformIdentityError {}

fn validate_transform_component(
    component: &'static str,
    value: &str,
) -> Result<(), TransformIdentityError> {
    if value.is_empty()
        || value.len() > MAX_TRANSFORM_COMPONENT_LEN
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(TransformIdentityError { component })
    } else {
        Ok(())
    }
}

/// Canonically frames every semantic contributor to a producer input hash.
///
/// The constructor requires the common transform, effect, output-kind, and
/// result-policy inputs. A concrete materializer must additionally add every
/// transform-specific property and resolved declared or transitive input.
pub struct InputFingerprintBuilder {
    producer_body: Vec<u8>,
    meaningful_properties: Vec<(String, Vec<u8>)>,
    declared_inputs: Vec<(DeclaredInput, Fingerprint)>,
    transitive_inputs: Vec<(ProducerId, Fingerprint, Fingerprint)>,
    transform: TransformIdentity,
    effect: EffectClass,
    output_kind: OutputKind,
    policy: ResultPolicy,
}

impl InputFingerprintBuilder {
    pub fn new(
        producer_body: impl AsRef<[u8]>,
        transform: TransformIdentity,
        effect: EffectClass,
        output_kind: OutputKind,
        policy: ResultPolicy,
    ) -> Self {
        Self {
            producer_body: producer_body.as_ref().to_vec(),
            meaningful_properties: Vec::new(),
            declared_inputs: Vec::new(),
            transitive_inputs: Vec::new(),
            transform,
            effect,
            output_kind,
            policy,
        }
    }

    pub fn meaningful_property(mut self, key: impl Into<String>, value: impl AsRef<[u8]>) -> Self {
        self.meaningful_properties
            .push((key.into().to_lowercase(), value.as_ref().to_vec()));
        self
    }

    /// Adds one resolved declaration; repeated declarations remain significant.
    pub fn declared_input(mut self, input: DeclaredInput, fingerprint: Fingerprint) -> Self {
        self.declared_inputs.push((input, fingerprint));
        self
    }

    /// Includes both sides of a dependency's successful provenance.
    pub fn transitive_input(
        mut self,
        producer: ProducerId,
        input: Fingerprint,
        output: Fingerprint,
    ) -> Self {
        self.transitive_inputs.push((producer, input, output));
        self
    }

    pub fn finish(mut self) -> Fingerprint {
        self.meaningful_properties.sort();
        self.declared_inputs.sort();
        self.transitive_inputs.sort();

        let mut builder = FingerprintBuilder::new("maki.materialization.input.v1");
        builder
            .field("producer-body", self.producer_body)
            .field("transform-kind", self.transform.kind())
            .field("transform-version", self.transform.version())
            .field("executor", self.transform.executor())
            .field(
                "executor-config",
                self.transform.executor_config().as_bytes(),
            )
            .field("effect", self.effect.as_str())
            .field("output-kind", self.output_kind.as_str())
            .field("result-policy", self.policy.as_str())
            .field(
                "property-count",
                (self.meaningful_properties.len() as u64).to_be_bytes(),
            );
        for (key, value) in self.meaningful_properties {
            builder
                .field("property-key", key)
                .field("property-value", value);
        }
        builder.field(
            "declared-input-count",
            (self.declared_inputs.len() as u64).to_be_bytes(),
        );
        for (input, fingerprint) in self.declared_inputs {
            builder
                .field("declared-input-kind", input.kind().as_str())
                .field("declared-input-value", input.value())
                .field("declared-input-fingerprint", fingerprint.as_bytes());
        }
        builder.field(
            "transitive-input-count",
            (self.transitive_inputs.len() as u64).to_be_bytes(),
        );
        for (producer, input, output) in self.transitive_inputs {
            builder
                .field("transitive-producer", producer.as_str())
                .field("transitive-input", input.as_bytes())
                .field("transitive-output", output.as_bytes());
        }
        builder.finish()
    }
}

pub fn output_fingerprint(output: impl AsRef<[u8]>) -> Fingerprint {
    let mut builder = FingerprintBuilder::new("maki.materialization.output.v1");
    builder.field("output", output);
    builder.finish()
}

pub fn executor_config_fingerprint(config: impl AsRef<[u8]>) -> Fingerprint {
    let mut builder = FingerprintBuilder::new("maki.materialization.executor-config.v1");
    builder.field("config", config);
    builder.finish()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRegion {
    pub path: PathBuf,
    pub span: SourceSpan,
}

impl SourceRegion {
    pub fn new(path: impl Into<PathBuf>, span: SourceSpan) -> Self {
        Self {
            path: path.into(),
            span,
        }
    }
}

/// Whether the current semantic input can be fingerprinted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentInput {
    Available(Fingerprint),
    /// A declared prerequisite is unavailable, so reconciliation cannot run.
    Blocked,
    /// The current snapshot does not contain enough information to compare it.
    Unverifiable,
}

/// One collected producer and all of its declared semantic inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationProducer {
    pub id: ProducerId,
    pub region: SourceRegion,
    pub current_input: CurrentInput,
    pub transform: TransformIdentity,
    pub effect: EffectClass,
    pub output_kind: OutputKind,
    pub policy: ResultPolicy,
    pub declared_inputs: Vec<DeclaredInput>,
}

impl MaterializationProducer {
    pub fn dependencies(&self) -> impl Iterator<Item = &ProducerId> {
        self.declared_inputs
            .iter()
            .filter_map(DeclaredInput::producer_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedArtifact {
    pub region: SourceRegion,
    pub generated_from: ProducerId,
    pub current_output: Option<Fingerprint>,
    pub successful_provenance: Option<SuccessfulProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessfulProvenance {
    pub input: Fingerprint,
    pub output: Fingerprint,
    pub transform: TransformIdentity,
    pub effect: EffectClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionSurface {
    Formatter,
    Check,
    LspDiagnostics,
    Save,
    Update,
    LspCommand,
    EvaluatorExport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionAuthorization {
    pub explicit: bool,
    pub trusted_workspace: bool,
    pub granted_capabilities: BTreeSet<EffectClass>,
}

/// Returns whether a trusted effect may execute on a particular surface.
///
/// Formatting, checking, and diagnostic publication never execute a
/// materializer. Save is restricted to an explicitly opted-in pure transform.
/// Explicit update/command/export entrypoints may run non-pure work only in a
/// trusted workspace with a grant for the exact trusted effect class.
pub fn effect_is_authorized(
    surface: ExecutionSurface,
    effect: EffectClass,
    authorization: &ExecutionAuthorization,
) -> bool {
    match surface {
        ExecutionSurface::Formatter
        | ExecutionSurface::Check
        | ExecutionSurface::LspDiagnostics => false,
        ExecutionSurface::Save => authorization.explicit && effect.is_pure(),
        ExecutionSurface::Update
        | ExecutionSurface::LspCommand
        | ExecutionSurface::EvaluatorExport => {
            authorization.explicit
                && (effect.is_pure()
                    || (authorization.trusted_workspace
                        && authorization.granted_capabilities.contains(&effect)))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Association {
    Matched,
    Missing,
    Orphan,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    StaleInput,
    ModifiedOutput,
    Blocked,
    Unverifiable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyState {
    Managed,
    Frozen,
    Volatile,
}

impl From<ResultPolicy> for PolicyState {
    fn from(policy: ResultPolicy) -> Self {
        match policy {
            ResultPolicy::Frozen => Self::Frozen,
            ResultPolicy::Volatile => Self::Volatile,
            ResultPolicy::Replace
            | ResultPolicy::Append
            | ResultPolicy::Manual
            | ResultPolicy::Cache => Self::Managed,
        }
    }
}

/// A stable single-label projection for protocol and summary surfaces.
///
/// The richer association, freshness, and policy axes remain available on the
/// report. Projection precedence is: ambiguous, orphan, missing, modified
/// output, blocked, frozen, volatile, unverifiable, stale input, then fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FreshnessState {
    Fresh,
    Missing,
    StaleInput,
    ModifiedOutput,
    Orphan,
    Ambiguous,
    Blocked,
    Unverifiable,
    Frozen,
    Volatile,
}

impl FreshnessState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Missing => "missing",
            Self::StaleInput => "stale-input",
            Self::ModifiedOutput => "modified-output",
            Self::Orphan => "orphan",
            Self::Ambiguous => "ambiguous",
            Self::Blocked => "blocked",
            Self::Unverifiable => "unverifiable",
            Self::Frozen => "frozen",
            Self::Volatile => "volatile",
        }
    }
}

impl fmt::Display for FreshnessState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationEvaluation {
    pub producer_regions: Vec<SourceRegion>,
    pub artifact_regions: Vec<SourceRegion>,
    pub dependencies: Vec<ProducerId>,
    pub association: Association,
    /// Freshness before dependency state is propagated.
    pub direct_freshness: Freshness,
    /// Freshness after deterministic transitive dependency propagation.
    pub freshness: Freshness,
    pub policy: PolicyState,
    pub primary_state: FreshnessState,
}

impl MaterializationEvaluation {
    pub fn has_duplicate_producers(&self) -> bool {
        self.producer_regions.len() > 1
    }

    pub fn has_duplicate_artifacts(&self) -> bool {
        self.artifact_regions.len() > 1
    }

    pub fn has_orphan_artifact(&self) -> bool {
        self.producer_regions.is_empty() && !self.artifact_regions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MaterializationEvaluationReport {
    entries: BTreeMap<ProducerId, MaterializationEvaluation>,
}

impl MaterializationEvaluationReport {
    pub fn entries(&self) -> &BTreeMap<ProducerId, MaterializationEvaluation> {
        &self.entries
    }

    pub fn get(&self, id: &ProducerId) -> Option<&MaterializationEvaluation> {
        self.entries.get(id)
    }
}

#[derive(Default)]
struct EvaluationInput {
    producers: Vec<MaterializationProducer>,
    artifacts: Vec<MaterializedArtifact>,
}

struct PendingEvaluation {
    producer_regions: Vec<SourceRegion>,
    artifact_regions: Vec<SourceRegion>,
    dependencies: Vec<ProducerId>,
    association: Association,
    direct_freshness: Freshness,
    direct_resolution: FreshnessResolution,
    policy: PolicyState,
}

pub fn evaluate_materializations(
    producers: impl IntoIterator<Item = MaterializationProducer>,
    artifacts: impl IntoIterator<Item = MaterializedArtifact>,
) -> MaterializationEvaluationReport {
    let mut grouped = BTreeMap::<ProducerId, EvaluationInput>::new();
    for producer in producers {
        grouped
            .entry(producer.id.clone())
            .or_default()
            .producers
            .push(producer);
    }
    for artifact in artifacts {
        grouped
            .entry(artifact.generated_from.clone())
            .or_default()
            .artifacts
            .push(artifact);
    }
    for input in grouped.values_mut() {
        input
            .producers
            .sort_by(|left, right| left.region.cmp(&right.region));
        input
            .artifacts
            .sort_by(|left, right| left.region.cmp(&right.region));
    }

    let pending = grouped
        .iter()
        .map(|(id, input)| (id.clone(), pending_evaluation(input)))
        .collect::<BTreeMap<_, _>>();
    let cycle_nodes = materialization_cycle_nodes(&pending);
    let resolved = resolve_freshnesses(&pending, &cycle_nodes);

    let entries = pending
        .into_iter()
        .map(|(id, pending)| {
            let freshness = resolved
                .get(&id)
                .copied()
                .unwrap_or_else(FreshnessResolution::blocked)
                .freshness();
            let primary_state = primary_state(pending.association, freshness, pending.policy);
            (
                id,
                MaterializationEvaluation {
                    producer_regions: pending.producer_regions,
                    artifact_regions: pending.artifact_regions,
                    dependencies: pending.dependencies,
                    association: pending.association,
                    direct_freshness: pending.direct_freshness,
                    freshness,
                    policy: pending.policy,
                    primary_state,
                },
            )
        })
        .collect();

    MaterializationEvaluationReport { entries }
}

fn pending_evaluation(input: &EvaluationInput) -> PendingEvaluation {
    let producer_regions = input
        .producers
        .iter()
        .map(|producer| producer.region.clone())
        .collect();
    let artifact_regions = input
        .artifacts
        .iter()
        .map(|artifact| artifact.region.clone())
        .collect();
    let association = match (input.producers.len(), input.artifacts.len()) {
        (1, 1) => Association::Matched,
        (1, 0) => Association::Missing,
        (0, 1) => Association::Orphan,
        _ => Association::Ambiguous,
    };
    let producer = (input.producers.len() == 1).then(|| &input.producers[0]);
    let artifact = (input.artifacts.len() == 1).then(|| &input.artifacts[0]);
    let policy = producer
        .map(|producer| PolicyState::from(producer.policy))
        .unwrap_or(PolicyState::Managed);
    let mut dependencies = producer
        .map(|producer| producer.dependencies().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    dependencies.sort();
    dependencies.dedup();
    let direct_resolution = match (association, producer, artifact) {
        (Association::Matched, Some(producer), Some(artifact)) => {
            direct_freshness_resolution(producer, artifact, policy)
        }
        _ => FreshnessResolution::blocked(),
    };

    PendingEvaluation {
        producer_regions,
        artifact_regions,
        dependencies,
        association,
        direct_freshness: direct_resolution.freshness(),
        direct_resolution,
        policy,
    }
}

fn direct_freshness_resolution(
    producer: &MaterializationProducer,
    artifact: &MaterializedArtifact,
    policy: PolicyState,
) -> FreshnessResolution {
    let mut resolution = FreshnessResolution::default();
    match producer.current_input {
        CurrentInput::Available(_) => {}
        CurrentInput::Blocked => resolution.blocked = true,
        CurrentInput::Unverifiable => resolution.unverifiable = true,
    }

    match artifact.successful_provenance.as_ref() {
        Some(provenance) => {
            match artifact.current_output {
                Some(current_output) if current_output != provenance.output => {
                    resolution.modified_output = true;
                }
                Some(_) => {}
                None => resolution.unverifiable = true,
            }
            if matches!(
                producer.current_input,
                CurrentInput::Available(current_input) if current_input != provenance.input
            ) || producer.transform != provenance.transform
                || producer.effect != provenance.effect
            {
                resolution.stale_input = true;
            }
        }
        None => resolution.unverifiable = true,
    }
    if policy == PolicyState::Volatile && resolution.freshness() == Freshness::Fresh {
        resolution.unverifiable = true;
    }
    resolution
}

#[derive(Debug, Clone, Copy, Default)]
struct FreshnessResolution {
    stale_input: bool,
    modified_output: bool,
    blocked: bool,
    unverifiable: bool,
    volatile: bool,
}

impl FreshnessResolution {
    fn for_entry(entry: &PendingEvaluation, is_cycle_member: bool) -> Self {
        let mut resolution = entry.direct_resolution;
        resolution.blocked |= is_cycle_member;
        resolution.volatile = entry.policy == PolicyState::Volatile;
        resolution
    }

    fn blocked() -> Self {
        Self {
            blocked: true,
            ..Self::default()
        }
    }

    fn include_dependency(&mut self, dependency: Self) {
        self.stale_input |= dependency.stale_input || dependency.modified_output;
        self.blocked |= dependency.blocked;
        self.unverifiable |= dependency.unverifiable || dependency.volatile;
    }

    fn freshness(self) -> Freshness {
        if self.modified_output {
            Freshness::ModifiedOutput
        } else if self.blocked {
            Freshness::Blocked
        } else if self.unverifiable {
            Freshness::Unverifiable
        } else if self.stale_input {
            Freshness::StaleInput
        } else {
            Freshness::Fresh
        }
    }
}

fn resolve_freshnesses(
    entries: &BTreeMap<ProducerId, PendingEvaluation>,
    cycle_nodes: &BTreeSet<ProducerId>,
) -> BTreeMap<ProducerId, FreshnessResolution> {
    let active = entries
        .iter()
        .filter(|(id, entry)| {
            entry.association == Association::Matched && !cycle_nodes.contains(*id)
        })
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let mut resolved = entries
        .iter()
        .filter(|(id, _)| !active.contains(*id))
        .map(|(id, entry)| {
            (
                id.clone(),
                FreshnessResolution::for_entry(entry, cycle_nodes.contains(id)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut remaining_dependencies = BTreeMap::new();
    let mut dependents = BTreeMap::<ProducerId, Vec<ProducerId>>::new();

    for id in &active {
        let entry = &entries[id];
        let mut active_dependencies = 0;
        for dependency in &entry.dependencies {
            if active.contains(dependency) {
                active_dependencies += 1;
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
        remaining_dependencies.insert(id.clone(), active_dependencies);
    }

    let mut ready = remaining_dependencies
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    while let Some(id) = ready.pop_first() {
        let entry = &entries[&id];
        let mut resolution = FreshnessResolution::for_entry(entry, false);
        for dependency in &entry.dependencies {
            resolution.include_dependency(
                resolved
                    .get(dependency)
                    .copied()
                    .unwrap_or_else(FreshnessResolution::blocked),
            );
        }
        resolved.insert(id.clone(), resolution);

        for dependent in dependents.get(&id).into_iter().flatten() {
            let remaining = remaining_dependencies
                .get_mut(dependent)
                .expect("active dependent must have a dependency count");
            *remaining -= 1;
            if *remaining == 0 {
                ready.insert(dependent.clone());
            }
        }
    }

    debug_assert_eq!(resolved.len(), entries.len());
    for id in active {
        resolved
            .entry(id.clone())
            .or_insert_with(|| FreshnessResolution::for_entry(&entries[&id], true));
    }
    resolved
}

fn primary_state(
    association: Association,
    freshness: Freshness,
    policy: PolicyState,
) -> FreshnessState {
    match association {
        Association::Ambiguous => FreshnessState::Ambiguous,
        Association::Orphan => FreshnessState::Orphan,
        Association::Missing => FreshnessState::Missing,
        Association::Matched => match freshness {
            Freshness::ModifiedOutput => FreshnessState::ModifiedOutput,
            Freshness::Blocked => FreshnessState::Blocked,
            _ if policy == PolicyState::Frozen => FreshnessState::Frozen,
            _ if policy == PolicyState::Volatile => FreshnessState::Volatile,
            Freshness::Unverifiable => FreshnessState::Unverifiable,
            Freshness::StaleInput => FreshnessState::StaleInput,
            Freshness::Fresh => FreshnessState::Fresh,
        },
    }
}

fn materialization_cycle_nodes(
    entries: &BTreeMap<ProducerId, PendingEvaluation>,
) -> BTreeSet<ProducerId> {
    let graph = entries
        .iter()
        .filter(|(_, entry)| entry.producer_regions.len() == 1)
        .map(|(id, entry)| {
            let dependencies = entry
                .dependencies
                .iter()
                .filter(|dependency| {
                    entries
                        .get(*dependency)
                        .is_some_and(|entry| entry.producer_regions.len() == 1)
                })
                .cloned()
                .collect();
            (id.clone(), dependencies)
        })
        .collect::<BTreeMap<_, Vec<_>>>();
    let mut reverse_graph = graph
        .keys()
        .cloned()
        .map(|id| (id, Vec::new()))
        .collect::<BTreeMap<_, Vec<_>>>();
    for (id, dependencies) in &graph {
        for dependency in dependencies {
            reverse_graph
                .get_mut(dependency)
                .expect("graph dependency must be a graph node")
                .push(id.clone());
        }
    }

    let mut visited = BTreeSet::new();
    let mut finishing_order = Vec::with_capacity(graph.len());
    for start in graph.keys() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut stack = vec![(start.clone(), 0)];
        while let Some((current, next_dependency)) = stack.last_mut() {
            let dependencies = &graph[current];
            if *next_dependency == dependencies.len() {
                let (finished, _) = stack.pop().expect("DFS stack must not be empty");
                finishing_order.push(finished);
                continue;
            }

            let dependency = dependencies[*next_dependency].clone();
            *next_dependency += 1;
            if visited.insert(dependency.clone()) {
                stack.push((dependency, 0));
            }
        }
    }

    let mut assigned = BTreeSet::new();
    let mut cycle_nodes = BTreeSet::new();
    for start in finishing_order.into_iter().rev() {
        if !assigned.insert(start.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            component.push(current.clone());
            for dependent in reverse_graph[&current].iter().rev() {
                if assigned.insert(dependent.clone()) {
                    stack.push(dependent.clone());
                }
            }
        }

        let is_cycle =
            component.len() > 1 || graph[&component[0]].binary_search(&component[0]).is_ok();
        if is_cycle {
            cycle_nodes.extend(component);
        }
    }
    cycle_nodes
}

/// Exact source text that a reconcile plan was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePrecondition {
    pub path: PathBuf,
    pub original: String,
}

impl SourcePrecondition {
    pub fn new(path: impl Into<PathBuf>, original: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            original: original.into(),
        }
    }
}

/// One source replacement. Its byte span is relative to the exact precondition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceEdit {
    pub path: PathBuf,
    pub span: SourceSpan,
    pub replacement: String,
}

impl SourceEdit {
    pub fn new(path: impl Into<PathBuf>, span: SourceSpan, replacement: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            span,
            replacement: replacement.into(),
        }
    }
}

/// A validated, deterministic set of source changes for one snapshot revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilePlan {
    revision: SnapshotRevision,
    preconditions: BTreeMap<PathBuf, String>,
    effect: EffectClass,
    edits: Vec<SourceEdit>,
}

impl ReconcilePlan {
    pub fn new(
        revision: SnapshotRevision,
        effect: EffectClass,
        preconditions: impl IntoIterator<Item = SourcePrecondition>,
        edits: impl IntoIterator<Item = SourceEdit>,
    ) -> Result<Self, ReconcilePlanError> {
        let mut indexed_preconditions = BTreeMap::new();
        for precondition in preconditions {
            let path = normalized_project_relative_path(&precondition.path).ok_or_else(|| {
                ReconcilePlanError::InvalidPath {
                    path: precondition.path.clone(),
                }
            })?;
            if indexed_preconditions
                .insert(path.clone(), precondition.original)
                .is_some()
            {
                return Err(ReconcilePlanError::DuplicatePrecondition { path });
            }
        }

        let mut edits = edits.into_iter().collect::<Vec<_>>();
        for edit in &mut edits {
            edit.path = normalized_project_relative_path(&edit.path).ok_or_else(|| {
                ReconcilePlanError::InvalidPath {
                    path: edit.path.clone(),
                }
            })?;
        }
        edits.sort();
        for edit in &edits {
            let original = indexed_preconditions.get(&edit.path).ok_or_else(|| {
                ReconcilePlanError::MissingPrecondition {
                    path: edit.path.clone(),
                }
            })?;
            validate_edit(original, edit).map_err(ReconcilePlanError::InvalidEdit)?;
        }
        for pair in edits.windows(2) {
            let [left, right] = pair else {
                unreachable!("a two-element window always has two elements");
            };
            if edits_conflict(left, right) {
                return Err(ReconcilePlanError::ConflictingEdits {
                    first: SourceRegion::new(left.path.clone(), left.span),
                    second: SourceRegion::new(right.path.clone(), right.span),
                });
            }
        }

        edits.retain(|edit| {
            let original = &indexed_preconditions[&edit.path];
            original[edit.span.start..edit.span.end] != edit.replacement
        });

        Ok(Self {
            revision,
            preconditions: indexed_preconditions,
            effect,
            edits,
        })
    }

    pub fn revision(&self) -> SnapshotRevision {
        self.revision
    }

    pub fn effect(&self) -> EffectClass {
        self.effect
    }

    pub fn preconditions(&self) -> &BTreeMap<PathBuf, String> {
        &self.preconditions
    }

    pub fn edits(&self) -> &[SourceEdit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Applies all edits in memory, or returns an error without changing the input.
    pub fn apply(
        &self,
        snapshot: &ProjectSnapshot,
    ) -> Result<ProjectSnapshot, ReconcileApplyError> {
        if snapshot.revision() != self.revision {
            return Err(ReconcileApplyError::StaleRevision {
                expected: self.revision,
                actual: snapshot.revision(),
            });
        }

        for (path, expected) in &self.preconditions {
            let actual = snapshot
                .source(path)
                .ok_or_else(|| ReconcileApplyError::MissingSource { path: path.clone() })?;
            if actual != expected {
                return Err(ReconcileApplyError::SourceChanged { path: path.clone() });
            }
        }
        for edit in &self.edits {
            let source =
                snapshot
                    .source(&edit.path)
                    .ok_or_else(|| ReconcileApplyError::MissingSource {
                        path: edit.path.clone(),
                    })?;
            validate_edit(source, edit).map_err(ReconcileApplyError::InvalidEdit)?;
        }

        if self.edits.is_empty() {
            return Ok(snapshot.clone());
        }

        let mut sources = snapshot
            .source_paths()
            .filter_map(|path| {
                snapshot
                    .source(path)
                    .map(|source| (path.to_path_buf(), source.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        for edit in self.edits.iter().rev() {
            let source = sources
                .get_mut(&edit.path)
                .expect("validated edit source must be present");
            source.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }

        Ok(ProjectSnapshot::compile(sources))
    }
}

fn normalized_project_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn edits_conflict(left: &SourceEdit, right: &SourceEdit) -> bool {
    left.path == right.path
        && (left.span.start == right.span.start || left.span.end > right.span.start)
}

fn validate_edit(source: &str, edit: &SourceEdit) -> Result<(), InvalidSourceEdit> {
    if edit.span.start > edit.span.end || edit.span.end > source.len() {
        return Err(InvalidSourceEdit::OutOfBounds {
            region: SourceRegion::new(edit.path.clone(), edit.span),
            source_len: source.len(),
        });
    }
    if !source.is_char_boundary(edit.span.start) {
        return Err(InvalidSourceEdit::NotCharBoundary {
            path: edit.path.clone(),
            offset: edit.span.start,
        });
    }
    if !source.is_char_boundary(edit.span.end) {
        return Err(InvalidSourceEdit::NotCharBoundary {
            path: edit.path.clone(),
            offset: edit.span.end,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidSourceEdit {
    OutOfBounds {
        region: SourceRegion,
        source_len: usize,
    },
    NotCharBoundary {
        path: PathBuf,
        offset: usize,
    },
}

impl fmt::Display for InvalidSourceEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { region, source_len } => write!(
                formatter,
                "edit {}:{}..{} is outside source length {source_len}",
                region.path.display(),
                region.span.start,
                region.span.end
            ),
            Self::NotCharBoundary { path, offset } => write!(
                formatter,
                "edit offset {offset} in {} is not a UTF-8 boundary",
                path.display()
            ),
        }
    }
}

impl std::error::Error for InvalidSourceEdit {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePlanError {
    InvalidPath {
        path: PathBuf,
    },
    DuplicatePrecondition {
        path: PathBuf,
    },
    MissingPrecondition {
        path: PathBuf,
    },
    InvalidEdit(InvalidSourceEdit),
    ConflictingEdits {
        first: SourceRegion,
        second: SourceRegion,
    },
}

impl fmt::Display for ReconcilePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { path } => write!(
                formatter,
                "source path {} must be non-empty and project-relative without parent components",
                path.display()
            ),
            Self::DuplicatePrecondition { path } => {
                write!(
                    formatter,
                    "duplicate source precondition for {}",
                    path.display()
                )
            }
            Self::MissingPrecondition { path } => {
                write!(
                    formatter,
                    "missing source precondition for {}",
                    path.display()
                )
            }
            Self::InvalidEdit(error) => error.fmt(formatter),
            Self::ConflictingEdits { first, second } => write!(
                formatter,
                "conflicting edits in {} at {}..{} and {}..{}",
                first.path.display(),
                first.span.start,
                first.span.end,
                second.span.start,
                second.span.end
            ),
        }
    }
}

impl std::error::Error for ReconcilePlanError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileApplyError {
    StaleRevision {
        expected: SnapshotRevision,
        actual: SnapshotRevision,
    },
    MissingSource {
        path: PathBuf,
    },
    SourceChanged {
        path: PathBuf,
    },
    InvalidEdit(InvalidSourceEdit),
}

impl fmt::Display for ReconcileApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "stale snapshot revision: expected {}, got {}",
                expected.get(),
                actual.get()
            ),
            Self::MissingSource { path } => {
                write!(formatter, "source {} is no longer present", path.display())
            }
            Self::SourceChanged { path } => {
                write!(
                    formatter,
                    "source {} changed since planning",
                    path.display()
                )
            }
            Self::InvalidEdit(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ReconcileApplyError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn producer_id(value: &str) -> ProducerId {
        value.parse().unwrap()
    }

    fn fingerprint(byte: u8) -> Fingerprint {
        Fingerprint::from_bytes([byte; 32])
    }

    fn transform(version: &str, config: u8) -> TransformIdentity {
        TransformIdentity::new("test", version, "builtin", fingerprint(config)).unwrap()
    }

    fn region(path: &str, start: usize) -> SourceRegion {
        SourceRegion::new(path, SourceSpan::new(start, start + 1))
    }

    fn producer(
        name: &str,
        input: Option<Fingerprint>,
        policy: ResultPolicy,
        dependencies: &[&str],
    ) -> MaterializationProducer {
        MaterializationProducer {
            id: producer_id(name),
            region: region(&format!("{name}.maki"), 0),
            current_input: input.map_or(CurrentInput::Unverifiable, CurrentInput::Available),
            transform: transform("1", 9),
            effect: EffectClass::PureProject,
            output_kind: OutputKind::Text,
            policy,
            declared_inputs: dependencies
                .iter()
                .map(|dependency| DeclaredInput::producer(producer_id(dependency)))
                .collect(),
        }
    }

    fn artifact(
        name: &str,
        current_output: Option<Fingerprint>,
        provenance: Option<SuccessfulProvenance>,
    ) -> MaterializedArtifact {
        MaterializedArtifact {
            region: region(&format!("{name}-result.maki"), 2),
            generated_from: producer_id(name),
            current_output,
            successful_provenance: provenance,
        }
    }

    fn provenance(input: u8, output: u8) -> SuccessfulProvenance {
        SuccessfulProvenance {
            input: fingerprint(input),
            output: fingerprint(output),
            transform: transform("1", 9),
            effect: EffectClass::PureProject,
        }
    }

    fn fresh_pair(
        name: &str,
        policy: ResultPolicy,
        dependencies: &[&str],
    ) -> (MaterializationProducer, MaterializedArtifact) {
        (
            producer(name, Some(fingerprint(1)), policy, dependencies),
            artifact(name, Some(fingerprint(2)), Some(provenance(1, 2))),
        )
    }

    fn snapshot(sources: &[(&str, &str)]) -> ProjectSnapshot {
        ProjectSnapshot::compile(
            sources
                .iter()
                .map(|(path, source)| (PathBuf::from(path), (*source).to_string()))
                .collect(),
        )
    }

    #[test]
    fn property_names_are_exact_and_success_is_explicit() {
        assert_eq!(PRODUCER_ID_PROPERTY, "producer-id");
        assert_eq!(RESULT_POLICY_PROPERTY, "result-policy");
        assert_eq!(MATERIALIZE_INPUT_PROPERTY, "materialize-input");
        assert_eq!(EFFECT_PROPERTY, "effect");
        assert_eq!(OUTPUT_KIND_PROPERTY, "output-kind");
        assert_eq!(GENERATED_FROM_PROPERTY, "generated-from");
        assert_eq!(GENERATED_INPUT_PROPERTY, "generated-input");
        assert_eq!(GENERATED_OUTPUT_PROPERTY, "generated-output");
        assert_eq!(GENERATED_TRANSFORM_PROPERTY, "generated-transform");
        assert_eq!(GENERATED_VERSION_PROPERTY, "generated-version");
        assert_eq!(GENERATED_EXECUTOR_PROPERTY, "generated-executor");
        assert_eq!(
            GENERATED_EXECUTOR_CONFIG_PROPERTY,
            "generated-executor-config"
        );
        assert_eq!(GENERATED_EFFECT_PROPERTY, "generated-effect");
        assert_eq!(GENERATED_STATUS_PROPERTY, "generated-status");
        assert_eq!(GENERATED_STATUS_SUCCESS, "success");
    }

    #[test]
    fn scalar_contracts_round_trip_and_reject_unsafe_spellings() {
        assert_eq!(producer_id("team.report_2").to_string(), "team.report_2");
        for invalid in ["", "-leading", "has space", "경로", "a/b"] {
            assert!(invalid.parse::<ProducerId>().is_err(), "{invalid}");
        }

        for policy in [
            ResultPolicy::Replace,
            ResultPolicy::Append,
            ResultPolicy::Frozen,
            ResultPolicy::Manual,
            ResultPolicy::Volatile,
            ResultPolicy::Cache,
        ] {
            assert_eq!(policy.to_string().parse::<ResultPolicy>().unwrap(), policy);
        }
        for effect in [
            EffectClass::PureDocument,
            EffectClass::PureProject,
            EffectClass::FilesystemRead,
            EffectClass::Process,
            EffectClass::Network,
            EffectClass::ClockRandomSecrets,
        ] {
            assert_eq!(effect.to_string().parse::<EffectClass>().unwrap(), effect);
        }

        for kind in [
            OutputKind::Text,
            OutputKind::Maki,
            OutputKind::Table,
            OutputKind::File,
            "plugin/chart-v2".parse().unwrap(),
        ] {
            assert_eq!(kind.to_string().parse::<OutputKind>().unwrap(), kind);
        }
        for invalid in ["custom", "/name", "plugin/", "a/b/c", "Plugin/chart"] {
            assert!(invalid.parse::<OutputKind>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn declared_inputs_preserve_kind_and_repeatable_values() {
        let inputs = [
            ("producer:upstream", "producer", "upstream"),
            ("block:daily/task", "block", "daily/task"),
            ("table:ledger", "table", "ledger"),
            ("config:timezone", "config", "timezone"),
            ("file:data/input.csv", "file", "data/input.csv"),
        ];
        for (source, kind, value) in inputs {
            let parsed = source.parse::<DeclaredInput>().unwrap();
            assert_eq!(parsed.kind().as_str(), kind);
            assert_eq!(parsed.value(), value);
            assert_eq!(parsed.to_string(), source);
        }
        for invalid in ["block", "unknown:x", "block:", "block: x", "producer:-x"] {
            assert!(invalid.parse::<DeclaredInput>().is_err(), "{invalid}");
        }
        assert!(DeclaredInput::new(DeclaredInputKind::File, " untrimmed").is_err());
        let producer = DeclaredInput::producer(producer_id("upstream"));
        assert_eq!(producer.producer_id(), Some(&producer_id("upstream")));
    }

    #[test]
    fn fingerprints_use_strict_lowercase_sha256_encoding_and_framing() {
        let value = fingerprint(0xab);
        let encoded = value.to_string();
        assert_eq!(encoded, format!("sha256:{}", "ab".repeat(32)));
        assert_eq!(encoded.parse::<Fingerprint>().unwrap(), value);
        assert!(encoded.to_uppercase().parse::<Fingerprint>().is_err());
        assert!("sha256:00".parse::<Fingerprint>().is_err());

        let mut first = FingerprintBuilder::new("test");
        first.field("a", b"bc");
        let mut second = FingerprintBuilder::new("test");
        second.field("ab", b"c");
        let mut other_domain = FingerprintBuilder::new("other");
        other_domain.field("a", b"bc");
        assert_ne!(first.finish(), second.finish());
        let mut same_field = FingerprintBuilder::new("test");
        same_field.field("a", b"bc");
        assert_ne!(same_field.finish(), other_domain.finish());
        assert_eq!(
            output_fingerprint(b"hello").to_string(),
            "sha256:a0420cfe06da5806cda2e7eee1ba9ccc3ab6fe1db9aab29e7f038f142947dfdd"
        );
    }

    fn semantic_input(reverse: bool) -> Fingerprint {
        let properties = if reverse {
            [
                ("mode", b"strict".as_slice()),
                ("title", b"Report".as_slice()),
            ]
        } else {
            [
                ("title", b"Report".as_slice()),
                ("mode", b"strict".as_slice()),
            ]
        };
        let declared = if reverse {
            [
                ("file:data.csv", fingerprint(4)),
                ("config:locale", fingerprint(3)),
            ]
        } else {
            [
                ("config:locale", fingerprint(3)),
                ("file:data.csv", fingerprint(4)),
            ]
        };
        let transitive = if reverse {
            [
                ("z", fingerprint(7), fingerprint(8)),
                ("a", fingerprint(5), fingerprint(6)),
            ]
        } else {
            [
                ("a", fingerprint(5), fingerprint(6)),
                ("z", fingerprint(7), fingerprint(8)),
            ]
        };
        let mut builder = InputFingerprintBuilder::new(
            b"producer body",
            transform("1", 9),
            EffectClass::PureProject,
            OutputKind::Table,
            ResultPolicy::Replace,
        );
        for (key, value) in properties {
            builder = builder.meaningful_property(key, value);
        }
        for (input, hash) in declared {
            builder = builder.declared_input(input.parse().unwrap(), hash);
        }
        for (id, input, output) in transitive {
            builder = builder.transitive_input(producer_id(id), input, output);
        }
        builder.finish()
    }

    #[test]
    fn semantic_input_fingerprint_is_order_independent_and_complete() {
        assert_eq!(semantic_input(false), semantic_input(true));

        let baseline = InputFingerprintBuilder::new(
            b"producer body",
            transform("1", 9),
            EffectClass::PureProject,
            OutputKind::Table,
            ResultPolicy::Replace,
        )
        .meaningful_property("mode", b"strict")
        .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
        .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
        .finish();
        let variants = [
            InputFingerprintBuilder::new(
                b"changed body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("2", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 10),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Append,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::Network,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Text,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"permissive")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(7))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(7), fingerprint(6))
            .finish(),
            InputFingerprintBuilder::new(
                b"producer body",
                transform("1", 9),
                EffectClass::PureProject,
                OutputKind::Table,
                ResultPolicy::Replace,
            )
            .meaningful_property("mode", b"strict")
            .declared_input("file:data.csv".parse().unwrap(), fingerprint(4))
            .transitive_input(producer_id("upstream"), fingerprint(5), fingerprint(7))
            .finish(),
        ];
        for variant in variants {
            assert_ne!(variant, baseline);
        }
        assert_ne!(output_fingerprint(b"x"), output_fingerprint(b"y"));
        assert_ne!(
            executor_config_fingerprint(b"x"),
            executor_config_fingerprint(b"y")
        );
    }

    #[test]
    fn execution_policy_never_runs_diagnostics_and_requires_exact_grants() {
        let explicit = ExecutionAuthorization {
            explicit: true,
            ..ExecutionAuthorization::default()
        };
        for surface in [
            ExecutionSurface::Formatter,
            ExecutionSurface::Check,
            ExecutionSurface::LspDiagnostics,
        ] {
            assert!(!effect_is_authorized(
                surface,
                EffectClass::PureDocument,
                &explicit
            ));
        }
        assert!(effect_is_authorized(
            ExecutionSurface::Save,
            EffectClass::PureProject,
            &explicit
        ));
        assert!(!effect_is_authorized(
            ExecutionSurface::Save,
            EffectClass::Network,
            &explicit
        ));
        assert!(!effect_is_authorized(
            ExecutionSurface::Update,
            EffectClass::PureDocument,
            &ExecutionAuthorization::default()
        ));

        let trusted = ExecutionAuthorization {
            explicit: true,
            trusted_workspace: true,
            granted_capabilities: BTreeSet::from([EffectClass::Network]),
        };
        assert!(effect_is_authorized(
            ExecutionSurface::Update,
            EffectClass::Network,
            &trusted
        ));
        assert!(!effect_is_authorized(
            ExecutionSurface::Update,
            EffectClass::Process,
            &trusted
        ));
        let untrusted = ExecutionAuthorization {
            trusted_workspace: false,
            ..trusted
        };
        assert!(!effect_is_authorized(
            ExecutionSurface::LspCommand,
            EffectClass::Network,
            &untrusted
        ));
    }

    #[test]
    fn evaluation_detects_association_drift_and_policy_axes() {
        let (fresh_producer, fresh_artifact) = fresh_pair("fresh", ResultPolicy::Replace, &[]);
        let missing_producer =
            producer("missing", Some(fingerprint(1)), ResultPolicy::Replace, &[]);
        let orphan_artifact = artifact("orphan", Some(fingerprint(2)), Some(provenance(1, 2)));

        let duplicate_a = producer(
            "duplicate",
            Some(fingerprint(1)),
            ResultPolicy::Replace,
            &[],
        );
        let mut duplicate_b = duplicate_a.clone();
        duplicate_b.region = region("a-first.maki", 0);
        let duplicate_artifact =
            artifact("duplicate", Some(fingerprint(2)), Some(provenance(1, 2)));

        let (result_producer, result_a) = fresh_pair("results", ResultPolicy::Replace, &[]);
        let mut result_b = result_a.clone();
        result_b.region = region("results-second.maki", 8);

        let stale_producer = producer("stale", Some(fingerprint(3)), ResultPolicy::Replace, &[]);
        let stale_artifact = artifact("stale", Some(fingerprint(2)), Some(provenance(1, 2)));
        let (mut transform_producer, transform_artifact) =
            fresh_pair("transform", ResultPolicy::Replace, &[]);
        transform_producer.transform = transform("2", 9);
        let (mut effect_producer, effect_artifact) =
            fresh_pair("effect", ResultPolicy::Replace, &[]);
        effect_producer.effect = EffectClass::Network;
        let modified_producer =
            producer("modified", Some(fingerprint(1)), ResultPolicy::Replace, &[]);
        let modified_artifact = artifact("modified", Some(fingerprint(3)), Some(provenance(1, 2)));
        let (frozen_producer, frozen_artifact) = fresh_pair("frozen", ResultPolicy::Frozen, &[]);
        let (volatile_producer, volatile_artifact) =
            fresh_pair("volatile", ResultPolicy::Volatile, &[]);
        let (mut blocked_producer, blocked_artifact) =
            fresh_pair("blocked-input", ResultPolicy::Replace, &[]);
        blocked_producer.current_input = CurrentInput::Blocked;
        let unverifiable_producer = producer("unknown", None, ResultPolicy::Replace, &[]);
        let unverifiable_artifact = artifact("unknown", None, None);

        let report = evaluate_materializations(
            [
                fresh_producer,
                missing_producer,
                duplicate_a,
                duplicate_b,
                result_producer,
                stale_producer,
                transform_producer,
                effect_producer,
                modified_producer,
                frozen_producer,
                volatile_producer,
                blocked_producer,
                unverifiable_producer,
            ],
            [
                fresh_artifact,
                orphan_artifact,
                duplicate_artifact,
                result_a,
                result_b,
                stale_artifact,
                transform_artifact,
                effect_artifact,
                modified_artifact,
                frozen_artifact,
                volatile_artifact,
                blocked_artifact,
                unverifiable_artifact,
            ],
        );

        assert_eq!(
            report.get(&producer_id("fresh")).unwrap().primary_state,
            FreshnessState::Fresh
        );
        assert_eq!(
            report.get(&producer_id("missing")).unwrap().association,
            Association::Missing
        );
        assert_eq!(
            report.get(&producer_id("missing")).unwrap().primary_state,
            FreshnessState::Missing
        );
        assert_eq!(
            report.get(&producer_id("orphan")).unwrap().association,
            Association::Orphan
        );
        assert!(
            report
                .get(&producer_id("orphan"))
                .unwrap()
                .has_orphan_artifact()
        );
        let duplicate = report.get(&producer_id("duplicate")).unwrap();
        assert_eq!(duplicate.association, Association::Ambiguous);
        assert!(duplicate.has_duplicate_producers());
        assert_eq!(
            duplicate.producer_regions[0].path,
            PathBuf::from("a-first.maki")
        );
        assert!(
            report
                .get(&producer_id("results"))
                .unwrap()
                .has_duplicate_artifacts()
        );
        for name in ["stale", "transform", "effect"] {
            assert_eq!(
                report.get(&producer_id(name)).unwrap().freshness,
                Freshness::StaleInput,
                "{name}"
            );
        }
        assert_eq!(
            report.get(&producer_id("modified")).unwrap().primary_state,
            FreshnessState::ModifiedOutput
        );
        let frozen = report.get(&producer_id("frozen")).unwrap();
        assert_eq!(frozen.policy, PolicyState::Frozen);
        assert_eq!(frozen.primary_state, FreshnessState::Frozen);
        let volatile = report.get(&producer_id("volatile")).unwrap();
        assert_eq!(volatile.policy, PolicyState::Volatile);
        assert_eq!(volatile.freshness, Freshness::Unverifiable);
        assert_eq!(volatile.primary_state, FreshnessState::Volatile);
        assert_eq!(
            report
                .get(&producer_id("blocked-input"))
                .unwrap()
                .primary_state,
            FreshnessState::Blocked
        );
        assert_eq!(
            report.get(&producer_id("unknown")).unwrap().primary_state,
            FreshnessState::Unverifiable
        );

        let ids = report
            .entries()
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn evaluation_propagates_dependencies_and_blocks_cycles() {
        let (mut stale, stale_artifact) = fresh_pair("stale", ResultPolicy::Replace, &[]);
        stale.current_input = CurrentInput::Available(fingerprint(3));
        let (stale_child, stale_child_artifact) =
            fresh_pair("stale-child", ResultPolicy::Replace, &["stale"]);

        let (modified, mut modified_artifact) = fresh_pair("modified", ResultPolicy::Replace, &[]);
        modified_artifact.current_output = Some(fingerprint(3));
        let (modified_child, modified_child_artifact) =
            fresh_pair("modified-child", ResultPolicy::Replace, &["modified"]);

        let (unknown, mut unknown_artifact) = fresh_pair("unknown", ResultPolicy::Replace, &[]);
        unknown_artifact.successful_provenance = None;
        let (unknown_child, unknown_child_artifact) =
            fresh_pair("unknown-child", ResultPolicy::Replace, &["unknown"]);
        let (volatile, volatile_artifact) = fresh_pair("volatile", ResultPolicy::Volatile, &[]);
        let (volatile_child, volatile_child_artifact) =
            fresh_pair("volatile-child", ResultPolicy::Replace, &["volatile"]);
        let (mut frozen, frozen_artifact) = fresh_pair("frozen", ResultPolicy::Frozen, &[]);
        frozen.current_input = CurrentInput::Available(fingerprint(3));
        let (frozen_child, frozen_child_artifact) =
            fresh_pair("frozen-child", ResultPolicy::Replace, &["frozen"]);

        let missing = producer("missing", Some(fingerprint(1)), ResultPolicy::Replace, &[]);
        let (missing_child, missing_child_artifact) =
            fresh_pair("missing-child", ResultPolicy::Replace, &["missing"]);
        let (absent_child, absent_child_artifact) =
            fresh_pair("absent-child", ResultPolicy::Replace, &["not-present"]);

        let ambiguous_a = producer(
            "ambiguous",
            Some(fingerprint(1)),
            ResultPolicy::Replace,
            &[],
        );
        let mut ambiguous_b = ambiguous_a.clone();
        ambiguous_b.region = region("ambiguous-copy.maki", 1);
        let ambiguous_artifact =
            artifact("ambiguous", Some(fingerprint(2)), Some(provenance(1, 2)));
        let (ambiguous_child, ambiguous_child_artifact) =
            fresh_pair("ambiguous-child", ResultPolicy::Replace, &["ambiguous"]);

        let (cycle_a, cycle_a_artifact) =
            fresh_pair("cycle-a", ResultPolicy::Replace, &["cycle-b"]);
        let (cycle_b, cycle_b_artifact) =
            fresh_pair("cycle-b", ResultPolicy::Replace, &["cycle-a"]);
        let (cycle_child, cycle_child_artifact) =
            fresh_pair("cycle-child", ResultPolicy::Replace, &["cycle-a"]);

        let report = evaluate_materializations(
            [
                stale,
                stale_child,
                modified,
                modified_child,
                unknown,
                unknown_child,
                volatile,
                volatile_child,
                frozen,
                frozen_child,
                missing,
                missing_child,
                absent_child,
                ambiguous_a,
                ambiguous_b,
                ambiguous_child,
                cycle_a,
                cycle_b,
                cycle_child,
            ],
            [
                stale_artifact,
                stale_child_artifact,
                modified_artifact,
                modified_child_artifact,
                unknown_artifact,
                unknown_child_artifact,
                volatile_artifact,
                volatile_child_artifact,
                frozen_artifact,
                frozen_child_artifact,
                missing_child_artifact,
                absent_child_artifact,
                ambiguous_artifact,
                ambiguous_child_artifact,
                cycle_a_artifact,
                cycle_b_artifact,
                cycle_child_artifact,
            ],
        );

        for name in ["stale-child", "modified-child", "frozen-child"] {
            assert_eq!(
                report.get(&producer_id(name)).unwrap().freshness,
                Freshness::StaleInput,
                "{name}"
            );
        }
        for name in ["unknown-child", "volatile-child"] {
            assert_eq!(
                report.get(&producer_id(name)).unwrap().freshness,
                Freshness::Unverifiable,
                "{name}"
            );
        }
        for name in [
            "missing-child",
            "absent-child",
            "ambiguous-child",
            "cycle-a",
            "cycle-b",
            "cycle-child",
        ] {
            assert_eq!(
                report.get(&producer_id(name)).unwrap().freshness,
                Freshness::Blocked,
                "{name}"
            );
        }
    }

    #[test]
    fn evaluation_keeps_modified_output_visible_while_cycles_block_dependents() {
        let (cycle_a, mut cycle_a_artifact) =
            fresh_pair("cycle-a", ResultPolicy::Replace, &["cycle-b"]);
        cycle_a_artifact.current_output = Some(fingerprint(3));
        let (cycle_b, cycle_b_artifact) =
            fresh_pair("cycle-b", ResultPolicy::Replace, &["cycle-a"]);
        let (cycle_child, cycle_child_artifact) =
            fresh_pair("cycle-child", ResultPolicy::Replace, &["cycle-a"]);

        let (volatile, volatile_artifact) =
            fresh_pair("volatile", ResultPolicy::Volatile, &["absent"]);
        let (volatile_child, volatile_child_artifact) =
            fresh_pair("volatile-child", ResultPolicy::Replace, &["volatile"]);
        let (mut blocked_modified, mut blocked_modified_artifact) =
            fresh_pair("blocked-modified", ResultPolicy::Replace, &[]);
        blocked_modified.current_input = CurrentInput::Blocked;
        blocked_modified_artifact.current_output = Some(fingerprint(3));
        let (blocked_modified_child, blocked_modified_child_artifact) = fresh_pair(
            "blocked-modified-child",
            ResultPolicy::Replace,
            &["blocked-modified"],
        );

        let report = evaluate_materializations(
            [
                cycle_a,
                cycle_b,
                cycle_child,
                volatile,
                volatile_child,
                blocked_modified,
                blocked_modified_child,
            ],
            [
                cycle_a_artifact,
                cycle_b_artifact,
                cycle_child_artifact,
                volatile_artifact,
                volatile_child_artifact,
                blocked_modified_artifact,
                blocked_modified_child_artifact,
            ],
        );

        let cycle_a = report.get(&producer_id("cycle-a")).unwrap();
        assert_eq!(cycle_a.direct_freshness, Freshness::ModifiedOutput);
        assert_eq!(cycle_a.freshness, Freshness::ModifiedOutput);
        assert_eq!(cycle_a.primary_state, FreshnessState::ModifiedOutput);
        for name in ["cycle-b", "cycle-child", "volatile", "volatile-child"] {
            assert_eq!(
                report.get(&producer_id(name)).unwrap().freshness,
                Freshness::Blocked,
                "{name}"
            );
        }
        assert_eq!(
            report
                .get(&producer_id("blocked-modified"))
                .unwrap()
                .freshness,
            Freshness::ModifiedOutput
        );
        assert_eq!(
            report
                .get(&producer_id("blocked-modified-child"))
                .unwrap()
                .freshness,
            Freshness::Blocked
        );
    }

    #[test]
    fn evaluation_handles_a_deep_dependency_chain_without_recursion() {
        const PRODUCER_COUNT: usize = 10_000;

        let mut producers = Vec::with_capacity(PRODUCER_COUNT);
        let mut artifacts = Vec::with_capacity(PRODUCER_COUNT);
        for index in 0..PRODUCER_COUNT {
            let name = format!("node-{index:05}");
            let (mut producer, artifact) = fresh_pair(&name, ResultPolicy::Replace, &[]);
            if index > 0 {
                producer.declared_inputs = vec![DeclaredInput::producer(producer_id(&format!(
                    "node-{:05}",
                    index - 1
                )))];
            }
            producers.push(producer);
            artifacts.push(artifact);
        }

        let report = evaluate_materializations(producers, artifacts);
        assert_eq!(report.entries().len(), PRODUCER_COUNT);
        assert_eq!(
            report.get(&producer_id("node-09999")).unwrap().freshness,
            Freshness::Fresh
        );
    }

    #[test]
    fn primary_projection_preserves_modified_and_policy_precedence() {
        let (mut modified, mut modified_artifact) =
            fresh_pair("modified", ResultPolicy::Replace, &["absent"]);
        modified.current_input = CurrentInput::Available(fingerprint(3));
        modified_artifact.current_output = Some(fingerprint(4));
        let (mut frozen, frozen_artifact) = fresh_pair("frozen", ResultPolicy::Frozen, &[]);
        frozen.current_input = CurrentInput::Available(fingerprint(3));
        let (mut volatile, volatile_artifact) = fresh_pair("volatile", ResultPolicy::Volatile, &[]);
        volatile.current_input = CurrentInput::Available(fingerprint(3));

        let report = evaluate_materializations(
            [modified, frozen, volatile],
            [modified_artifact, frozen_artifact, volatile_artifact],
        );
        let modified = report.get(&producer_id("modified")).unwrap();
        assert_eq!(modified.direct_freshness, Freshness::ModifiedOutput);
        assert_eq!(modified.freshness, Freshness::ModifiedOutput);
        assert_eq!(modified.primary_state, FreshnessState::ModifiedOutput);
        let frozen = report.get(&producer_id("frozen")).unwrap();
        assert_eq!(frozen.freshness, Freshness::StaleInput);
        assert_eq!(frozen.primary_state, FreshnessState::Frozen);
        let volatile = report.get(&producer_id("volatile")).unwrap();
        assert_eq!(volatile.freshness, Freshness::StaleInput);
        assert_eq!(volatile.primary_state, FreshnessState::Volatile);
    }

    #[test]
    fn reconcile_constructor_rejects_missing_duplicate_and_conflicting_inputs() {
        let project = snapshot(&[("a.maki", "abcdef")]);
        for path in ["", "/a.maki", "../a.maki"] {
            let invalid_path = ReconcilePlan::new(
                project.revision(),
                EffectClass::PureDocument,
                [SourcePrecondition::new(path, "")],
                [],
            );
            assert!(
                matches!(invalid_path, Err(ReconcilePlanError::InvalidPath { .. })),
                "{path}"
            );
        }
        let normalized = ReconcilePlan::new(
            project.revision(),
            EffectClass::PureDocument,
            [SourcePrecondition::new("./a//b.maki", "abcdef")],
            [SourceEdit::new("a/b.maki", SourceSpan::new(0, 1), "A")],
        )
        .unwrap();
        assert_eq!(
            normalized.preconditions().keys().next().unwrap(),
            Path::new("a/b.maki")
        );
        assert_eq!(normalized.edits()[0].path, Path::new("a/b.maki"));

        let duplicate = ReconcilePlan::new(
            project.revision(),
            EffectClass::PureDocument,
            [
                SourcePrecondition::new("a.maki", "abcdef"),
                SourcePrecondition::new("a.maki", "abcdef"),
            ],
            [],
        );
        assert!(matches!(
            duplicate,
            Err(ReconcilePlanError::DuplicatePrecondition { .. })
        ));

        let missing = ReconcilePlan::new(
            project.revision(),
            EffectClass::PureDocument,
            [],
            [SourceEdit::new("a.maki", SourceSpan::new(0, 1), "A")],
        );
        assert!(matches!(
            missing,
            Err(ReconcilePlanError::MissingPrecondition { .. })
        ));

        let conflict = ReconcilePlan::new(
            project.revision(),
            EffectClass::PureDocument,
            [SourcePrecondition::new("a.maki", "abcdef")],
            [
                SourceEdit::new("a.maki", SourceSpan::new(1, 4), "x"),
                SourceEdit::new("a.maki", SourceSpan::new(3, 5), "y"),
            ],
        );
        assert!(matches!(
            conflict,
            Err(ReconcilePlanError::ConflictingEdits { .. })
        ));

        let unicode = ReconcilePlan::new(
            snapshot(&[("u.maki", "é")]).revision(),
            EffectClass::PureDocument,
            [SourcePrecondition::new("u.maki", "é")],
            [SourceEdit::new("u.maki", SourceSpan::new(1, 2), "x")],
        );
        assert!(matches!(
            unicode,
            Err(ReconcilePlanError::InvalidEdit(
                InvalidSourceEdit::NotCharBoundary { .. }
            ))
        ));
    }

    #[test]
    fn reconcile_applies_sorted_edits_to_multiple_paths_atomically() {
        let original = snapshot(&[("a.maki", "abcdef"), ("b.maki", "uvwxyz")]);
        let plan = ReconcilePlan::new(
            original.revision(),
            EffectClass::PureProject,
            [
                SourcePrecondition::new("b.maki", "uvwxyz"),
                SourcePrecondition::new("a.maki", "abcdef"),
            ],
            [
                SourceEdit::new("b.maki", SourceSpan::new(0, 2), "UV"),
                SourceEdit::new("a.maki", SourceSpan::new(4, 6), "EF"),
                SourceEdit::new("a.maki", SourceSpan::new(0, 2), "AB"),
            ],
        )
        .unwrap();

        assert_eq!(plan.effect(), EffectClass::PureProject);
        assert!(plan.edits().windows(2).all(|pair| pair[0] < pair[1]));
        let updated = plan.apply(&original).unwrap();
        assert_eq!(
            updated.source(PathBuf::from("a.maki").as_path()),
            Some("ABcdEF")
        );
        assert_eq!(
            updated.source(PathBuf::from("b.maki").as_path()),
            Some("UVwxyz")
        );
        assert_ne!(updated.revision(), original.revision());
        assert_eq!(
            original.source(PathBuf::from("a.maki").as_path()),
            Some("abcdef")
        );
        assert_eq!(
            original.source(PathBuf::from("b.maki").as_path()),
            Some("uvwxyz")
        );
    }

    #[test]
    fn reconcile_rejects_stale_revision_and_exact_source_without_partial_results() {
        let original = snapshot(&[("a.maki", "abc"), ("b.maki", "xyz")]);
        let wrong_revision = snapshot(&[("a.maki", "abc"), ("b.maki", "xyz")]);
        let revision_plan = ReconcilePlan::new(
            original.revision(),
            EffectClass::PureProject,
            [SourcePrecondition::new("a.maki", "abc")],
            [SourceEdit::new("a.maki", SourceSpan::new(0, 1), "A")],
        )
        .unwrap();
        assert!(matches!(
            revision_plan.apply(&wrong_revision),
            Err(ReconcileApplyError::StaleRevision { .. })
        ));

        let source_plan = ReconcilePlan::new(
            original.revision(),
            EffectClass::PureProject,
            [
                SourcePrecondition::new("a.maki", "abc"),
                SourcePrecondition::new("b.maki", "xyZ"),
            ],
            [
                SourceEdit::new("a.maki", SourceSpan::new(0, 1), "A"),
                SourceEdit::new("b.maki", SourceSpan::new(0, 1), "X"),
            ],
        )
        .unwrap();
        assert!(matches!(
            source_plan.apply(&original),
            Err(ReconcileApplyError::SourceChanged { path })
                if path == std::path::Path::new("b.maki")
        ));
        assert_eq!(
            original.source(PathBuf::from("a.maki").as_path()),
            Some("abc")
        );
        assert_eq!(
            original.source(PathBuf::from("b.maki").as_path()),
            Some("xyz")
        );
    }

    #[test]
    fn reconcile_no_op_is_idempotent_and_keeps_revision() {
        let original = snapshot(&[("a.maki", "abc")]);
        let plan = ReconcilePlan::new(
            original.revision(),
            EffectClass::PureDocument,
            [SourcePrecondition::new("a.maki", "abc")],
            [SourceEdit::new("a.maki", SourceSpan::new(0, 3), "abc")],
        )
        .unwrap();
        assert!(plan.is_empty());
        let once = plan.apply(&original).unwrap();
        let twice = plan.apply(&once).unwrap();
        assert_eq!(once, original);
        assert_eq!(twice, original);
        assert_eq!(twice.revision(), original.revision());
    }
}
