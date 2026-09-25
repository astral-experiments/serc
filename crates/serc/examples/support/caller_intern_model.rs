//! Opt-in replay of a reviewed caller's retained POSIX path components.
//!
//! The sealed profile remains native provenance. The effective compiler state is
//! an independent pre-path baseline, a source-derived compiler phase, and the
//! current filename's retained components. Python 3.13 uses guaranteed-positive
//! membership with an unknown default; Python 3.12 retains its complete-state
//! contract. V3 additionally requires a separately reviewed absence proof and
//! masks source-derived codec requests back to unknown. No state accumulates
//! between files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serc::{CompileOptions, InitialInternState, PythonVersion};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::native_output_cache::{Artifact, artifact, digest, regular_bytes};

const MODE: &str = "pathlib-posix-source-bytes-v1";
const IMPLEMENTATION: &str = "cpython-pathlib-posix-parts-v1";
const ORACLE_SHA256: &str = "6812e016e7e871700001ad9f8f832c5998d553bc33c84afb1745dbd664a965fb";
const BASELINE_SHA256: &str = "af650692baf658821c1bd969fc0016d3e94300fb74755bcb1058486994c3b669";
const FIXED_PHASE_SHA256: &str = "1bd4edaf3f6b04c859224ecc819c2ec5d09e59a60207c2085e28c7dc5fcc8ccc";
const PATHLIB_SHA256: &str = "10ba48ae8063cfe7589436041d0d4628c05f427c7709cf8e75e486f8930cc32c";
const POSIXPATH_SHA256: &str = "03825681086638649a43480954f7f6a16b4da3bd41ece956864ae96e4f795cd9";
const SEALED_PROFILE_SHA256: &str =
    "63c0a5a8e13abd26c756b9790081ed0d98f1c9d0960ae933b565847c38e3b51d";
const EXECUTABLE_SHA256: &str = "febd623a8152b648d90174369fb7eb121be1ed653fa0fa5d72e3b512cce8aa8e";
const RUNTIME_INPUTS_SHA256: &str =
    "5c5010b0998ee3e501130a05b28c7fca9c38574302d9b2b9a56195271c915c9a";
const BOUNDARY: &str = "sys_intern:entry:first-call-from-pinned-pathlib-_parse_path";
const POST_BOUNDARY: &str = "Py_CompileStringObject:entry";

const PARTIAL_MODE: &str = "pathlib-posix-source-bytes-partial-v2";
const PARTIAL_BASELINE_SEMANTICS: &str = "guaranteed-positive";
const UNKNOWN_DEFAULT: &str = "unknown";
const DEPENDENCY_POLICY: &str = "fail-if-marshal-depends-on-unknown";
// Both reviewed certificates establish the same 804 retained names. The second
// restores lost proof artifacts from pinned sources; all other identity checks apply.
const RETAINED_NATIVE_POSITIVE_SHA256S: &[&str] = &[
    "7c4035bbc11d5b0de867d7ac53cd11b1848287134fcf6e3c2ca7aad5da575486",
    "c87be884f078331cdbea552567d7fdd8119e016ef2ba0e6d788e07f925c0a2c9",
];
const ABSENCE_MODE: &str = "pathlib-posix-source-bytes-partial-v3";
const EXACT_ABSENCE_MODE: &str = "pathlib-posix-source-bytes-partial-v4";
const COOKIE_EVIDENCE_SHA256: &str =
    "0a9dfd17ddeaa8de27d33f74c19d7a61048443fa75b6747f1094a0e16542aab5";
const REQUIRED_ABSENT_PATH: &str =
    "/home/dev-user/code/oss/serc-compat-oracles-20260921/python313/runtime/lib/python313.zip";

struct AbsencePins {
    coverage_sha256: &'static str,
    names_sha256: &'static str,
}

// Activation requires independently reviewed global source coverage. Pending
// evidence must never supply its own admission decision.
const REVIEWED_ABSENCE_PINS: Option<AbsencePins> = Some(AbsencePins {
    coverage_sha256: "f7943b4ff0e97e2352c18351b7eaa1ae6ec0639bd9c6171a94507e30270d8679",
    names_sha256: "b1b6f7c8ed0a4243804ecb025dd248c8bda13fd623c905328eb5769845a67421",
});

// Exact membership is a separate theorem; the substring contract remains intact.
const REVIEWED_EXACT_ABSENCE_PINS: Option<AbsencePins> = Some(AbsencePins {
    coverage_sha256: "53cfba242d811edd264847e0ea6bb5deb646b019314dd67a8de5ba497a218673",
    names_sha256: "d0b132489186397e58e0fcb6364e53aee85e39fa7e5201a6654b24e844217f48",
});

#[derive(Clone, Copy)]
enum AbsenceRule {
    Substrings,
    Exact,
}

impl AbsenceRule {
    fn for_mode(mode: &str) -> Option<Self> {
        match mode {
            ABSENCE_MODE => Some(Self::Substrings),
            EXACT_ABSENCE_MODE => Some(Self::Exact),
            _ => None,
        }
    }

    fn pins(self) -> Result<&'static AbsencePins, String> {
        match self {
            Self::Substrings => &REVIEWED_ABSENCE_PINS,
            Self::Exact => &REVIEWED_EXACT_ABSENCE_PINS,
        }
        .as_ref()
        .ok_or_else(|| {
            "caller absence model has no admitted global source-coverage proof".to_string()
        })
    }
}

struct ReviewedModel {
    schema_version: u32,
    mode: &'static str,
    commit: &'static str,
    baseline_sha256: &'static str,
    baseline_entries: usize,
    fixed_phase_sha256: &'static str,
    phase_entries: usize,
    pathlib_sha256: &'static str,
    posixpath_sha256: &'static str,
    sealed_profile_sha256: &'static str,
    executable_sha256: &'static str,
    runtime_inputs_sha256: &'static str,
    positive_evidence_sha256: Option<&'static str>,
}

fn reviewed_model(python_version: PythonVersion) -> Result<ReviewedModel, String> {
    match python_version {
        PythonVersion::Py312 => Ok(ReviewedModel {
            schema_version: 1,
            mode: MODE,
            commit: "2abcf904b8dac8c999d2b3aac76681abb333798a",
            baseline_sha256: BASELINE_SHA256,
            baseline_entries: 5494,
            fixed_phase_sha256: FIXED_PHASE_SHA256,
            phase_entries: 86,
            pathlib_sha256: PATHLIB_SHA256,
            posixpath_sha256: POSIXPATH_SHA256,
            sealed_profile_sha256: SEALED_PROFILE_SHA256,
            executable_sha256: EXECUTABLE_SHA256,
            runtime_inputs_sha256: RUNTIME_INPUTS_SHA256,
            positive_evidence_sha256: None,
        }),
        PythonVersion::Py313 => Ok(ReviewedModel {
            schema_version: 2,
            mode: PARTIAL_MODE,
            commit: "4061bc4c35f7c26f25264666d4ba083b93d2f6f9",
            // Source-derived static and guaranteed-import positives; observed
            // entries without that proof remain unknown.
            baseline_sha256: "b2ece4caddd2665d956420f6b307b9a7ee70fd5c65cbcf5eccb1b073b40fef6c",
            baseline_entries: 5168,
            fixed_phase_sha256: "e702d4a225b5ab84b4fb7d68eef77e502edf77c86df07ab3c9be422e0287e9ae",
            phase_entries: 87,
            pathlib_sha256: "0530b4186ec5845e000bedf5817ff1ac970b9a39a6891ba6789a1776abdea42b",
            posixpath_sha256: "69acb9e294987cd6056a2237048373cf8d6201faf1e43a49c38889b47d2fb28e",
            sealed_profile_sha256: "f95f2576abd453456de35dc9b78adbc6103687335b8ff94f7ece3449d795b70c",
            executable_sha256: "da775e5e09a4cfdd488a6d942c90b564dbbc3776ec8bf24f4a8ad0b4128fa3fa",
            runtime_inputs_sha256: "71d9b51fcb2a0cfe4fe1c00473b47e7f9646d397bb61785c38791f4017d38430",
            positive_evidence_sha256: Some(
                "be0e6b222a3154501f37be39527d5cb22ce0c189842ce3d1278ae30e6705c379",
            ),
        }),
        PythonVersion::Py314 | PythonVersion::Py315 => {
            Err("caller model has no reviewed contract for this target".to_string())
        }
    }
}

pub(super) struct LoadedModel {
    state: InitialInternState,
    source_cookie_masks: bool,
    pub(super) required_absent_paths: Vec<PathBuf>,
    pub(super) evidence: Value,
    pub(super) inputs: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema_version: u32,
    mode: String,
    target: Value,
    compiler_input_mode: String,
    oracle_sha256: String,
    sealed_profile: Artifact,
    executable: Artifact,
    runtime_inputs: Artifact,
    baseline: Artifact,
    fixed_phase: Artifact,
    model: Model,
    observations: Vec<Artifact>,
    baseline_semantics: Option<String>,
    unknown_default: Option<String>,
    dependency_policy: Option<String>,
    positive_baseline_evidence: Option<Artifact>,
    absence_evidence: Option<Artifact>,
    retained_native_positive_evidence: Option<Artifact>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Model {
    implementation: String,
    rust_sha256: String,
    pathlib: Artifact,
    posixpath: Artifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    schema_version: u32,
    mode: String,
    target: Value,
    baseline_sha256: String,
    fixed_phase_sha256: String,
    sealed_profile_sha256: String,
    executable_sha256: String,
    runtime_inputs_sha256: String,
    oracle_sha256: String,
    boundary: String,
    post_boundary: String,
    invariant: bool,
    baseline_invariant: Option<bool>,
    baseline_semantics: Option<String>,
    unknown_default: Option<String>,
    dependency_policy: Option<String>,
    positive_baseline_evidence: Option<Artifact>,
    controls: Vec<Artifact>,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InternEntry {
    pool: String,
    state: u8,
    codepoints: Vec<u32>,
}

/// Called only after the unchanged sealed-profile validator has accepted its
/// complete runtime closure. This loader binds that identity without treating
/// its neutral-path snapshot as the effective compiler state.
pub(super) fn load(
    path: &Path,
    sealed_profile_path: &Path,
    python_version: PythonVersion,
    source_bytes: bool,
    oracle: &str,
    assume_absent: bool,
) -> Result<LoadedModel, String> {
    let descriptor_bytes = regular_bytes(path).map_err(|error| error.to_string())?;
    let descriptor: Descriptor = read_json(&descriptor_bytes)?;
    descriptor.validate_contract(python_version, source_bytes, oracle)?;
    require(
        !assume_absent
            || (python_version == PythonVersion::Py313
                && descriptor.positive_baseline_evidence.is_some()),
        "assume-absent diagnostic requires a Python 3.13 guaranteed-positive caller model",
    )?;
    let reviewed = reviewed_model(python_version)?;
    require(
        descriptor.sealed_profile.path == sealed_profile_path,
        "caller model names a different sealed profile",
    )?;
    let mut inputs = vec![artifact(path, &descriptor_bytes)];
    let profile: Value = read_json(&read_artifact(&descriptor.sealed_profile, &mut inputs)?)?;
    require(
        profile["target"] == descriptor.target
            && profile["compiler_input_mode"] == descriptor.compiler_input_mode
            && profile["oracle"]["script_sha256"] == descriptor.oracle_sha256,
        "caller model target or oracle differs from the sealed profile",
    )?;
    for (name, item) in [
        ("executable", &descriptor.executable),
        ("runtime_inputs", &descriptor.runtime_inputs),
    ] {
        require(
            profile[name]["path"].as_str() == item.path.to_str()
                && profile[name]["sha256"] == item.sha256,
            "caller model executable or runtime identity differs from the sealed profile",
        )?;
        read_artifact(item, &mut inputs)?;
    }
    read_artifact(&descriptor.model.pathlib, &mut inputs)?;
    read_artifact(&descriptor.model.posixpath, &mut inputs)?;
    let mut baseline = read_entries(&read_artifact(&descriptor.baseline, &mut inputs)?)?;
    require(
        baseline.len() == reviewed.baseline_entries
            && baseline.contains("")
            && (0..=255).all(|point| baseline.contains(&char::from(point).to_string())),
        "caller baseline lacks its reviewed entries and singleton strings",
    )?;
    let phase = read_entries(&read_artifact(&descriptor.fixed_phase, &mut inputs)?)?;
    require(
        phase.len() == reviewed.phase_entries,
        "caller fixed compiler phase differs",
    )?;
    if let Some(guarantee) = &descriptor.positive_baseline_evidence {
        let guarantee: Value = read_json(&read_artifact(guarantee, &mut inputs)?)?;
        require(
            guarantee["target"] == descriptor.target
                && guarantee["baseline_sha256"] == descriptor.baseline.sha256,
            "caller positive-membership evidence names a different target or baseline",
        )?;
        let artifacts: Vec<Artifact> = serde_json::from_value(guarantee["artifacts"].clone())
            .map_err(|error| error.to_string())?;
        require(
            !artifacts.is_empty(),
            "caller positive-membership source evidence is missing",
        )?;
        for item in &artifacts {
            read_artifact(item, &mut inputs)?;
        }
    }
    for observation in &descriptor.observations {
        let observation: Observation = read_json(&read_artifact(observation, &mut inputs)?)?;
        observation.validate(&descriptor)?;
        for control in &observation.controls {
            let control: Value = read_json(&read_artifact(control, &mut inputs)?)?;
            let raw: Vec<Artifact> = serde_json::from_value(control["raw_artifacts"].clone())
                .map_err(|error| error.to_string())?;
            require(
                !raw.is_empty(),
                "caller control raw observation evidence is missing",
            )?;
            for item in &raw {
                read_artifact(item, &mut inputs)?;
            }
        }
        for item in &observation.artifacts {
            read_artifact(item, &mut inputs)?;
        }
    }
    if let Some(item) = &descriptor.retained_native_positive_evidence {
        baseline.extend(load_retained_native_positives(
            item,
            &descriptor,
            &mut inputs,
        )?);
    }
    let absence = descriptor
        .absence_evidence
        .as_ref()
        .map(|item| load_absence(item, &descriptor.target, &descriptor.mode, &mut inputs))
        .transpose()?;
    let has_absence_evidence = absence.is_some();
    let required_absent_paths = absence
        .as_ref()
        .map_or_else(Vec::new, |(_, paths)| paths.clone());
    let entries = baseline.into_iter().chain(phase);
    let partial = descriptor.positive_baseline_evidence.is_some();
    let (state, source_cookie_masks) = initial_state(
        entries,
        absence.map(|(names, _)| names),
        partial,
        assume_absent,
    );
    let mut evidence = json!({
        "mode": descriptor.mode,
        "descriptor": artifact(path, &descriptor_bytes),
        "target": descriptor.target,
        "oracle_sha256": descriptor.oracle_sha256,
        "sealed_profile": descriptor.sealed_profile,
        "baseline": descriptor.baseline,
        "fixed_phase": descriptor.fixed_phase,
        "model": descriptor.model,
        "observations": descriptor.observations,
        "effective_state": "complete-pre-path-baseline-plus-fixed-phase-plus-retained-path-components",
        "per_compilation_reset": true,
        "filename_contract": "original-literal-UTF8-POSIX-path-already-canonical-for-pathlib",
    });
    if partial {
        evidence["schema_version"] = json!(descriptor.schema_version);
        evidence["baseline_semantics"] = json!(descriptor.baseline_semantics);
        evidence["unknown_default"] = json!(descriptor.unknown_default);
        evidence["dependency_policy"] = json!(descriptor.dependency_policy);
        evidence["positive_baseline_evidence"] = json!(descriptor.positive_baseline_evidence);
        evidence["effective_state"] =
            json!("guaranteed-positive-baseline-plus-fixed-phase-plus-retained-path-components");
    }
    if has_absence_evidence {
        evidence["absence_evidence"] = json!(descriptor.absence_evidence);
        evidence["positive_component_contract"] = json!({
            "schema_version": 2, "mode": PARTIAL_MODE,
        });
        evidence["effective_state"] = json!(
            "guaranteed-positive-baseline-plus-fixed-phase-plus-retained-path-components-with-proved-absence-and-source-cookie-unknown-masks"
        );
    }
    if let Some(item) = &descriptor.retained_native_positive_evidence {
        evidence["retained_native_positive_evidence"] = json!(item);
    }
    if assume_absent {
        mark_assume_absent(&mut evidence);
    }
    Ok(LoadedModel {
        state,
        source_cookie_masks,
        required_absent_paths,
        evidence,
        inputs,
    })
}

fn initial_state(
    entries: impl IntoIterator<Item = String>,
    possible_names: Option<PossibleNames>,
    partial: bool,
    assume_absent: bool,
) -> (InitialInternState, bool) {
    if assume_absent {
        // Explicit diagnostic assumption: only supplied positives are retained.
        // No predicate, source cookie, or per-file result selects additional facts.
        (InitialInternState::from_complete(entries), false)
    } else if let Some(possible_names) = possible_names {
        (
            InitialInternState::from_partial_with_absence(entries, move |version, value| {
                possible_names.proves_absence(version, value)
            }),
            true,
        )
    } else if partial {
        (InitialInternState::from_partial(entries), false)
    } else {
        (InitialInternState::from_complete(entries), false)
    }
}

fn mark_assume_absent(evidence: &mut Value) {
    // Keep the admitted descriptor and its evidence as provenance. Neither an
    // absent table entry nor the retained absence proof justifies this heuristic.
    evidence["initial_unknown_assumption"] = json!("absent");
    evidence["assumption_status"] = json!("heuristic-not-source-proved");
    evidence["source_cookie_unknown_masks"] = json!(false);
    evidence["unknown_default"] = json!("assumed-absent");
    evidence["dependency_policy"] = json!("diagnostic-assume-absent");
    evidence["effective_state"] = json!(
        "guaranteed-positive-baseline-plus-fixed-phase-plus-retained-path-components-with-heuristic-assume-absent-default"
    );
}

impl Descriptor {
    fn validate_contract(
        &self,
        python_version: PythonVersion,
        source_bytes: bool,
        oracle: &str,
    ) -> Result<(), String> {
        require(
            cfg!(target_os = "linux"),
            "caller model requires the reviewed Linux POSIX platform",
        )?;
        let mut reviewed = reviewed_model(python_version)?;
        if python_version == PythonVersion::Py313 {
            match AbsenceRule::for_mode(&self.mode) {
                Some(AbsenceRule::Substrings) => {
                    reviewed.schema_version = 3;
                    reviewed.mode = ABSENCE_MODE;
                }
                Some(AbsenceRule::Exact) => {
                    reviewed.schema_version = 4;
                    reviewed.mode = EXACT_ABSENCE_MODE;
                }
                None => {}
            }
        }
        let target = python_version.target();
        require(
            target.commit == reviewed.commit
                && self.target
                    == json!({
                        "implementation": target.implementation,
                        "version": target.version_string(),
                        "tag": target.tag,
                        "commit": target.commit,
                        "magic_hex": target.magic_hex(),
                    }),
            "caller model CPython target differs from its reviewed implementation",
        )?;
        require(
            self.schema_version == reviewed.schema_version && self.mode == reviewed.mode,
            "unsupported caller model schema or mode",
        )?;
        require(
            source_bytes && self.compiler_input_mode == "source-bytes",
            "caller model requires source-bytes input",
        )?;
        require(
            self.oracle_sha256 == ORACLE_SHA256 && digest(oracle.as_bytes()) == ORACLE_SHA256,
            "caller model requires the unchanged reviewed oracle",
        )?;
        require(
            self.sealed_profile.sha256 == reviewed.sealed_profile_sha256
                && self.executable.sha256 == reviewed.executable_sha256
                && self.runtime_inputs.sha256 == reviewed.runtime_inputs_sha256,
            "caller model runtime or sealed profile has not been independently reviewed",
        )?;
        require(
            self.baseline.sha256 == reviewed.baseline_sha256
                && self.fixed_phase.sha256 == reviewed.fixed_phase_sha256,
            "unknown caller baseline or fixed compiler phase identity",
        )?;
        require(
            self.model.implementation == IMPLEMENTATION
                && self.model.rust_sha256 == digest(include_bytes!("caller_intern_model.rs"))
                && self.model.pathlib.sha256 == reviewed.pathlib_sha256
                && self.model.posixpath.sha256 == reviewed.posixpath_sha256,
            "unknown caller parsing implementation identity",
        )?;
        require(
            match reviewed.positive_evidence_sha256 {
                None => {
                    self.baseline_semantics.is_none()
                        && self.unknown_default.is_none()
                        && self.dependency_policy.is_none()
                        && self.positive_baseline_evidence.is_none()
                }
                Some(expected) => {
                    self.baseline_semantics.as_deref() == Some(PARTIAL_BASELINE_SEMANTICS)
                        && self.unknown_default.as_deref() == Some(UNKNOWN_DEFAULT)
                        && self.dependency_policy.as_deref() == Some(DEPENDENCY_POLICY)
                        && self
                            .positive_baseline_evidence
                            .as_ref()
                            .is_some_and(|artifact| artifact.sha256 == expected)
                }
            },
            "caller state semantics or guaranteed-positive evidence differs from the reviewed contract",
        )?;
        if let Some(item) = &self.retained_native_positive_evidence {
            require(
                python_version == PythonVersion::Py313 && self.positive_baseline_evidence.is_some(),
                "retained native positives require a Python 3.13 guaranteed-positive caller model",
            )?;
            require(
                RETAINED_NATIVE_POSITIVE_SHA256S.contains(&item.sha256.as_str()),
                "unadmitted retained native positive evidence",
            )?;
        }
        if let Some(rule) = AbsenceRule::for_mode(&self.mode) {
            let pins = rule.pins()?;
            require(
                self.absence_evidence
                    .as_ref()
                    .is_some_and(|item| item.sha256 == pins.coverage_sha256),
                "unknown caller absence-evidence identity",
            )?;
        } else {
            require(
                self.absence_evidence.is_none(),
                "caller absence evidence requires an absence model mode",
            )?;
        }
        require(
            !self.observations.is_empty(),
            "caller observation evidence is missing",
        )
    }
}

impl Observation {
    fn validate(&self, descriptor: &Descriptor) -> Result<(), String> {
        let membership_matches = match (
            &self.positive_baseline_evidence,
            &descriptor.positive_baseline_evidence,
        ) {
            (None, None) => self.invariant && self.baseline_invariant.is_none(),
            (Some(observed), Some(expected)) => {
                !self.invariant
                    && self.baseline_invariant == Some(true)
                    && observed.path == expected.path
                    && observed.sha256 == expected.sha256
                    && observed.bytes == expected.bytes
            }
            _ => false,
        };
        // Absence models compose the immutable V2 positive evidence. They do
        // not retag that observation as evidence for a separate absence theorem.
        let (schema_version, mode) = if AbsenceRule::for_mode(&descriptor.mode).is_some() {
            (2, PARTIAL_MODE)
        } else {
            (descriptor.schema_version, descriptor.mode.as_str())
        };
        require(
            self.schema_version == schema_version
                && self.mode == mode
                && self.target == descriptor.target
                && self.baseline_sha256 == descriptor.baseline.sha256
                && self.fixed_phase_sha256 == descriptor.fixed_phase.sha256
                && self.sealed_profile_sha256 == descriptor.sealed_profile.sha256
                && self.executable_sha256 == descriptor.executable.sha256
                && self.runtime_inputs_sha256 == descriptor.runtime_inputs.sha256
                && self.oracle_sha256 == descriptor.oracle_sha256
                && self.boundary == BOUNDARY
                && self.post_boundary == POST_BOUNDARY
                && self.baseline_semantics == descriptor.baseline_semantics
                && self.unknown_default == descriptor.unknown_default
                && self.dependency_policy == descriptor.dependency_policy
                && membership_matches
                && !self.controls.is_empty()
                && !self.artifacts.is_empty(),
            "caller observation does not bind the reviewed baseline, phase, and runtime boundaries",
        )
    }
}

impl LoadedModel {
    pub(super) fn options_for(
        &self,
        filename: &str,
        raw_source: &[u8],
        options: &CompileOptions,
    ) -> CompileOptions {
        let state = self.state.with_additional_strings(retained_parts(filename));
        let state = if self.source_cookie_masks {
            state.with_absence_exceptions(source_cookie_exceptions(raw_source))
        } else {
            state
        };
        CompileOptions {
            python_version: options.python_version,
            native_build_profile: options.native_build_profile,
            initial_intern_state: state,
        }
    }
}

/// Keep the original compiler filename. The oracle formats `str(path)`, so a
/// literal argument that would change during that operation is unsupported by
/// this mode and rejects the invocation before any files are compared.
pub(super) fn validate_filename(path: &Path) -> Result<(), String> {
    let filename = path
        .to_str()
        .ok_or("caller model requires UTF-8 filenames")?;
    require(
        !filename.contains('\0'),
        "caller model rejects NUL filenames",
    )?;
    require(
        canonical_filename(filename) == filename,
        "caller path model requires an original filename already canonical for pathlib; filename remapping is forbidden",
    )
}

// Both pinned 3.12 pathlib.py and 3.13 pathlib/_local.py use
// [sys.intern(str(x)) for x in rel.split(sep) if x and x != '.'].
// POSIX splitroot has no drive and removes only leading '/' characters from rel;
// filtering empty components therefore excludes either '/' or '//' root without
// interning it. '..', backslashes, and colon-containing components remain literal.
fn retained_parts(filename: &str) -> impl Iterator<Item = &str> {
    filename
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
}

fn canonical_filename(filename: &str) -> String {
    let root = if filename.starts_with("//") && !filename.starts_with("///") {
        "//"
    } else if filename.starts_with('/') {
        "/"
    } else {
        ""
    };
    let tail = retained_parts(filename).collect::<Vec<_>>().join("/");
    if root.is_empty() && tail.is_empty() {
        ".".to_string()
    } else {
        format!("{root}{tail}")
    }
}

// Family and review metadata is bound by the admitted aggregate hash. Its
// canonical names supplement the immutable baseline without rewriting it.
#[derive(Deserialize)]
struct RetainedNativePositiveEvidence {
    schema_version: u32,
    kind: String,
    target: Value,
    compiler_input_mode: String,
    oracle_sha256: String,
    sealed_profile_sha256: String,
    executable_sha256: String,
    runtime_inputs_sha256: String,
    names: Artifact,
    artifacts: Vec<Artifact>,
}

impl RetainedNativePositiveEvidence {
    fn validate(&self, descriptor: &Descriptor) -> Result<(), String> {
        require(
            self.schema_version == 1
                && self.kind == "retained-native-positive-v1"
                && self.target == descriptor.target
                && self.compiler_input_mode == descriptor.compiler_input_mode
                && self.oracle_sha256 == descriptor.oracle_sha256
                && self.sealed_profile_sha256 == descriptor.sealed_profile.sha256
                && self.executable_sha256 == descriptor.executable.sha256
                && self.runtime_inputs_sha256 == descriptor.runtime_inputs.sha256
                && !self.artifacts.is_empty(),
            "retained native positives differ from the admitted caller protocol",
        )
    }
}

fn load_retained_native_positives(
    item: &Artifact,
    descriptor: &Descriptor,
    inputs: &mut Vec<Artifact>,
) -> Result<Vec<String>, String> {
    let evidence: RetainedNativePositiveEvidence = read_json(&read_artifact(item, inputs)?)?;
    evidence.validate(descriptor)?;
    let names = read_retained_native_names(&read_artifact(&evidence.names, inputs)?)?;
    for artifact in &evidence.artifacts {
        read_artifact(artifact, inputs)?;
    }
    Ok(names)
}

fn read_retained_native_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    let names: Vec<String> = read_json(bytes)?;
    require(
        !names.is_empty()
            && names.iter().all(|name| !name.is_empty())
            && names.windows(2).all(|pair| pair[0] < pair[1]),
        "retained native names must be sorted unique nonempty strings",
    )?;
    Ok(names)
}

// Additional review metadata is allowed: the independently admitted aggregate
// hash binds the entire document, including its flat immutable artifact closure.
#[derive(Deserialize)]
struct AbsenceCoverage {
    schema_version: u32,
    target: Value,
    review_status: String,
    polarity: String,
    domain: String,
    matching: String,
    #[serde(default, deserialize_with = "present_field")]
    always_possible_prefixes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present_field")]
    substring_roots: Option<Artifact>,
    #[serde(default, deserialize_with = "present_field")]
    possible_names: Option<Artifact>,
    source_cookie_policy: String,
    source_cookie_evidence: Artifact,
    coverage_rows: Vec<Value>,
    conditions: Vec<Value>,
    unrestricted_inputs: Vec<Value>,
    required_absent_paths: Vec<PathBuf>,
    artifacts: Vec<Artifact>,
}

impl AbsenceCoverage {
    fn validate(
        &self,
        target: &Value,
        pins: &AbsencePins,
        rule: AbsenceRule,
    ) -> Result<(), String> {
        let matching = match rule {
            AbsenceRule::Substrings => {
                self.matching == "ascii-case-insensitive-substring"
                    && self
                        .always_possible_prefixes
                        .as_deref()
                        .is_some_and(|prefixes| prefixes == ["_"])
                    && self.possible_names.is_none()
            }
            AbsenceRule::Exact => {
                self.matching == "case-sensitive-exact"
                    && self.always_possible_prefixes.is_none()
                    && self.substring_roots.is_none()
            }
        };
        require(
            self.schema_version == 1
                && self.target == *target
                && self.review_status == "independently-reviewed-source-coverage"
                && self.polarity == "possible-present-superset"
                && self.domain == "nonempty-ascii-alnum-underscore"
                && matching
                && self
                    .names(rule)
                    .is_some_and(|item| item.sha256 == pins.names_sha256)
                && self.source_cookie_policy == "all-byte-offset-normalized-cookie-v1"
                && self.source_cookie_evidence.sha256 == COOKIE_EVIDENCE_SHA256
                && self.required_absent_paths.len() == 1
                && self.required_absent_paths[0] == Path::new(REQUIRED_ABSENT_PATH)
                && !self.coverage_rows.is_empty()
                && !self.conditions.is_empty()
                && !self.unrestricted_inputs.is_empty()
                && !self.artifacts.is_empty(),
            "caller absence coverage differs from the reviewed source contract",
        )
    }

    fn names(&self, rule: AbsenceRule) -> Option<&Artifact> {
        match rule {
            AbsenceRule::Substrings => self.substring_roots.as_ref(),
            AbsenceRule::Exact => self.possible_names.as_ref(),
        }
    }
}

// Missing optional policy fields are allowed; explicit null is not a policy.
fn present_field<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn load_absence(
    item: &Artifact,
    target: &Value,
    mode: &str,
    inputs: &mut Vec<Artifact>,
) -> Result<(PossibleNames, Vec<PathBuf>), String> {
    let rule = AbsenceRule::for_mode(mode).ok_or("unsupported caller absence model mode")?;
    let pins = rule.pins()?;
    require(
        item.sha256 == pins.coverage_sha256,
        "unknown caller absence-evidence identity",
    )?;
    let coverage: AbsenceCoverage = read_json(&read_artifact(item, inputs)?)?;
    coverage.validate(target, pins, rule)?;
    let names = coverage
        .names(rule)
        .ok_or("caller absence name evidence is missing")?;
    let possible_names = PossibleNames::read(&read_artifact(names, inputs)?, rule)?;
    let cookie: Value = read_json(&read_artifact(&coverage.source_cookie_evidence, inputs)?)?;
    require(
        cookie["schema"] == "serc-source-cookie-identifier-domain-v1"
            && cookie["query_domain"] == "[A-Za-z0-9_]*",
        "caller source-cookie evidence has a different domain",
    )?;
    for artifact in &coverage.artifacts {
        read_artifact(artifact, inputs)?;
    }
    validate_absent_paths(&coverage.required_absent_paths)?;
    Ok((possible_names, coverage.required_absent_paths))
}

enum PossibleNames {
    // Newlines outside the query domain prevent matches across separate roots.
    Substrings(String),
    Exact(BTreeSet<String>),
}

impl PossibleNames {
    fn read(bytes: &[u8], rule: AbsenceRule) -> Result<Self, String> {
        let names: Vec<String> = read_json(bytes)?;
        require(
            !names.is_empty()
                && names.iter().all(|name| {
                    ascii_word(name)
                        && (matches!(rule, AbsenceRule::Exact)
                            || !name.bytes().any(|byte| byte.is_ascii_uppercase()))
                })
                && names.windows(2).all(|pair| pair[0] < pair[1]),
            "caller possible names must be sorted unique ASCII words with the reviewed casing",
        )?;
        Ok(match rule {
            AbsenceRule::Substrings => Self::Substrings(names.join("\n")),
            AbsenceRule::Exact => Self::Exact(names.into_iter().collect()),
        })
    }

    fn proves_absence(&self, version: PythonVersion, value: &str) -> bool {
        if version != PythonVersion::Py313 || !ascii_word(value) {
            return false;
        }
        match self {
            Self::Substrings(names) => {
                !value.starts_with('_') && !names.contains(value.to_ascii_lowercase().as_str())
            }
            Self::Exact(names) => !names.contains(value),
        }
    }
}

fn ascii_word(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Enumerate possible codec requests at every raw byte offset, including later
/// physical lines that a diagnostic reread can inspect. The source only masks
/// absence: it never establishes a new positive or absent membership fact.
fn source_cookie_exceptions(source: &[u8]) -> BTreeSet<String> {
    let mut exceptions = BTreeSet::from([
        "utf_8".to_string(),
        "iso_8859_1".to_string(),
        "utf_8_sig".to_string(),
    ]);
    for (offset, marker) in source.windows(7).enumerate() {
        if &marker[..6] != b"coding" || !matches!(marker[6], b':' | b'=') {
            continue;
        }
        let mut start = offset + 7;
        while source
            .get(start)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            start += 1;
        }
        let mut end = start;
        while source
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'.' | b'-'))
        {
            end += 1;
        }
        let mut normalized = String::new();
        let mut punctuation = false;
        for &byte in &source[start..end] {
            if byte.is_ascii_alphanumeric() || byte == b'.' {
                if punctuation && !normalized.is_empty() {
                    normalized.push('_');
                }
                normalized.push(char::from(byte.to_ascii_lowercase()));
                punctuation = false;
            } else {
                punctuation = true;
            }
        }
        if ascii_word(&normalized) {
            exceptions.insert(normalized);
        }
    }
    exceptions
}

/// Absence is a runtime condition too. A dangling symlink, an unreadable path,
/// or an object appearing after loading invalidates the reviewed closure.
pub(super) fn validate_absent_paths(paths: &[PathBuf]) -> Result<(), String> {
    for path in paths {
        require(
            path.is_absolute(),
            "caller required-absent path must be absolute",
        )?;
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(format!(
                    "caller required-absent path is not absent: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn read_entries(bytes: &[u8]) -> Result<BTreeSet<String>, String> {
    let entries: Vec<InternEntry> = read_json(bytes)?;
    let mut strings = BTreeSet::new();
    for entry in entries {
        require(
            matches!(
                (entry.pool.as_str(), entry.state),
                ("static", 3) | ("dynamic", 1 | 2)
            ),
            "invalid caller intern pool/state",
        )?;
        let value = entry
            .codepoints
            .into_iter()
            .map(|point| {
                char::from_u32(point).ok_or("caller state contains a non-scalar codepoint")
            })
            .collect::<Result<String, _>>()?;
        require(strings.insert(value), "duplicate caller intern value")?;
    }
    Ok(strings)
}

fn read_artifact(item: &Artifact, inputs: &mut Vec<Artifact>) -> Result<Vec<u8>, String> {
    let bytes = item.read().map_err(|error| error.to_string())?;
    inputs.push(item.clone());
    Ok(bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Descriptor {
        let target = PythonVersion::Py312.target();
        let placeholder =
            json!({"path": "/evidence/artifact", "bytes": 0, "sha256": "placeholder"});
        let mut value = json!({
            "schema_version": 1, "mode": MODE,
            "target": {"implementation": target.implementation, "version": target.version_string(),
                "tag": target.tag, "commit": target.commit, "magic_hex": target.magic_hex()},
            "compiler_input_mode": "source-bytes", "oracle_sha256": ORACLE_SHA256,
            "sealed_profile": placeholder, "executable": placeholder, "runtime_inputs": placeholder,
            "baseline": placeholder, "fixed_phase": placeholder,
            "model": {"implementation": IMPLEMENTATION,
                "rust_sha256": digest(include_bytes!("caller_intern_model.rs")),
                "pathlib": placeholder, "posixpath": placeholder},
            "observations": [placeholder],
        });
        value["sealed_profile"]["sha256"] = json!(SEALED_PROFILE_SHA256);
        value["executable"]["sha256"] = json!(EXECUTABLE_SHA256);
        value["runtime_inputs"]["sha256"] = json!(RUNTIME_INPUTS_SHA256);
        value["baseline"]["sha256"] = json!(BASELINE_SHA256);
        value["fixed_phase"]["sha256"] = json!(FIXED_PHASE_SHA256);
        value["model"]["pathlib"]["sha256"] = json!(PATHLIB_SHA256);
        value["model"]["posixpath"]["sha256"] = json!(POSIXPATH_SHA256);
        serde_json::from_value(value).unwrap()
    }

    fn partial_descriptor() -> Descriptor {
        let mut descriptor = descriptor();
        let target = PythonVersion::Py313.target();
        let reviewed = reviewed_model(PythonVersion::Py313).unwrap();
        descriptor.schema_version = reviewed.schema_version;
        descriptor.mode = reviewed.mode.to_string();
        descriptor.target = json!({
            "implementation": target.implementation, "version": target.version_string(),
            "tag": target.tag, "commit": target.commit, "magic_hex": target.magic_hex(),
        });
        descriptor.baseline.sha256 = reviewed.baseline_sha256.to_string();
        descriptor.fixed_phase.sha256 = reviewed.fixed_phase_sha256.to_string();
        descriptor.sealed_profile.sha256 = reviewed.sealed_profile_sha256.to_string();
        descriptor.executable.sha256 = reviewed.executable_sha256.to_string();
        descriptor.runtime_inputs.sha256 = reviewed.runtime_inputs_sha256.to_string();
        descriptor.model.pathlib.sha256 = reviewed.pathlib_sha256.to_string();
        descriptor.model.posixpath.sha256 = reviewed.posixpath_sha256.to_string();
        descriptor.baseline_semantics = Some(PARTIAL_BASELINE_SEMANTICS.to_string());
        descriptor.unknown_default = Some(UNKNOWN_DEFAULT.to_string());
        descriptor.dependency_policy = Some(DEPENDENCY_POLICY.to_string());
        descriptor.positive_baseline_evidence = Some(Artifact {
            path: Path::new("/evidence/positive-membership.json").to_path_buf(),
            bytes: 0,
            sha256: reviewed.positive_evidence_sha256.unwrap().to_string(),
        });
        descriptor
    }

    fn observation(descriptor: &Descriptor) -> Observation {
        let partial = descriptor.positive_baseline_evidence.is_some();
        serde_json::from_value(json!({
            "schema_version": descriptor.schema_version, "mode": descriptor.mode,
            "target": descriptor.target, "baseline_sha256": descriptor.baseline.sha256,
            "fixed_phase_sha256": descriptor.fixed_phase.sha256,
            "sealed_profile_sha256": descriptor.sealed_profile.sha256,
            "executable_sha256": descriptor.executable.sha256,
            "runtime_inputs_sha256": descriptor.runtime_inputs.sha256,
            "oracle_sha256": descriptor.oracle_sha256,
            "boundary": BOUNDARY, "post_boundary": POST_BOUNDARY,
            "invariant": !partial, "baseline_invariant": partial.then_some(true),
            "baseline_semantics": descriptor.baseline_semantics,
            "unknown_default": descriptor.unknown_default,
            "dependency_policy": descriptor.dependency_policy,
            "positive_baseline_evidence": descriptor.positive_baseline_evidence,
            "controls": descriptor.observations, "artifacts": descriptor.observations,
        }))
        .unwrap()
    }

    #[test]
    fn partial_mode_requires_unknown_default_and_reviewed_positive_evidence() {
        let oracle = super::super::ORACLE;
        assert_eq!(
            partial_descriptor()
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_ok(),
            cfg!(target_os = "linux"),
        );
        for replacement in [None, Some("absent".to_string())] {
            let mut descriptor = partial_descriptor();
            descriptor.unknown_default = replacement;
            assert!(
                descriptor
                    .validate_contract(PythonVersion::Py313, true, oracle)
                    .is_err()
            );
        }
        let mut descriptor = partial_descriptor();
        descriptor.positive_baseline_evidence = None;
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_err()
        );
        let mut descriptor = partial_descriptor();
        descriptor
            .positive_baseline_evidence
            .as_mut()
            .unwrap()
            .sha256 = "unreviewed".to_string();
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_err()
        );
        let mut descriptor = partial_descriptor();
        descriptor.dependency_policy = Some("assume-absent".to_string());
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_err()
        );
    }

    #[test]
    fn partial_observations_preserve_the_complete_mode_invariant_requirement() {
        let complete = descriptor();
        let partial = partial_descriptor();
        let mut complete_observation = observation(&complete);
        let mut partial_observation = observation(&partial);
        assert!(complete_observation.validate(&complete).is_ok());
        assert!(partial_observation.validate(&partial).is_ok());
        complete_observation.invariant = false;
        complete_observation.baseline_invariant = Some(true);
        assert!(complete_observation.validate(&complete).is_err());
        partial_observation.invariant = true;
        assert!(partial_observation.validate(&partial).is_err());
        partial_observation.invariant = false;
        partial_observation.baseline_invariant = None;
        assert!(partial_observation.validate(&partial).is_err());
    }

    #[test]
    fn absence_mode_requires_admission_and_preserves_v2_observations() {
        let oracle = super::super::ORACLE;
        let mut descriptor = partial_descriptor();
        let positive_observation = observation(&descriptor);
        descriptor.schema_version = 3;
        descriptor.mode = ABSENCE_MODE.to_string();
        descriptor.absence_evidence = Some(descriptor.baseline.clone());
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_err()
        );
        assert!(positive_observation.validate(&descriptor).is_ok());
        assert!(observation(&descriptor).validate(&descriptor).is_err());
        descriptor.absence_evidence.as_mut().unwrap().sha256 = REVIEWED_ABSENCE_PINS
            .as_ref()
            .unwrap()
            .coverage_sha256
            .to_string();
        assert_eq!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_ok(),
            cfg!(target_os = "linux"),
        );
        descriptor.absence_evidence = None;
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, oracle)
                .is_err()
        );
        for mut descriptor in [self::descriptor(), partial_descriptor()] {
            descriptor.absence_evidence = Some(descriptor.baseline.clone());
            let version = if descriptor.schema_version == 1 {
                PythonVersion::Py312
            } else {
                PythonVersion::Py313
            };
            assert!(descriptor.validate_contract(version, true, oracle).is_err());
        }
    }

    #[test]
    fn absence_coverage_rejects_pending_review_and_changed_rules() {
        let descriptor = partial_descriptor();
        let roots = json!({"path": "/evidence/roots", "bytes": 1, "sha256": "synthetic roots"});
        let value = json!({
            "schema_version": 1, "target": descriptor.target,
            "review_status": "independently-reviewed-source-coverage",
            "polarity": "possible-present-superset", "domain": "nonempty-ascii-alnum-underscore",
            "matching": "ascii-case-insensitive-substring", "always_possible_prefixes": ["_"],
            "substring_roots": roots,
            "source_cookie_policy": "all-byte-offset-normalized-cookie-v1",
            "source_cookie_evidence": {"path": "/evidence/cookie", "bytes": 1, "sha256": COOKIE_EVIDENCE_SHA256},
            "coverage_rows": [{}], "conditions": ["synthetic condition"],
            "unrestricted_inputs": [{}], "required_absent_paths": [REQUIRED_ABSENT_PATH],
            "artifacts": [roots],
        });
        let pins = AbsencePins {
            coverage_sha256: "synthetic aggregate",
            names_sha256: "synthetic roots",
        };
        let check = |value: Value| {
            serde_json::from_value::<AbsenceCoverage>(value)
                .unwrap()
                .validate(&descriptor.target, &pins, AbsenceRule::Substrings)
        };
        assert!(check(value.clone()).is_ok());
        for (key, replacement) in [
            ("review_status", json!("pending-independent-global-review")),
            ("polarity", json!("absent-list")),
            ("always_possible_prefixes", json!([])),
            ("source_cookie_policy", json!("first-two-lines-only")),
            ("artifacts", json!([])),
            ("required_absent_paths", json!([])),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            assert!(check(changed).is_err(), "{key}");
        }
    }

    #[test]
    fn possible_names_use_only_the_admitted_substring_domain() {
        let names =
            PossibleNames::read(br#"["alpha", "beta_name"]"#, AbsenceRule::Substrings).unwrap();
        for value in ["ALP", "ha", "name", "_arbitrary", "", "alpha.beta", "é"] {
            assert!(
                !names.proves_absence(PythonVersion::Py313, value),
                "{value}"
            );
        }
        for value in ["habe", "gamma", "999"] {
            assert!(names.proves_absence(PythonVersion::Py313, value), "{value}");
            assert!(!names.proves_absence(PythonVersion::Py312, value));
        }
        for invalid in [
            r#"[]"#,
            r#"[""]"#,
            r#"["A"]"#,
            r#"["a","a"]"#,
            r#"["b","a"]"#,
            r#"["a.b"]"#,
            r#"["é"]"#,
        ] {
            assert!(
                PossibleNames::read(invalid.as_bytes(), AbsenceRule::Substrings).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn exact_mode_has_separate_admission_and_keeps_v2_positive_evidence() {
        let mut descriptor = partial_descriptor();
        let positive_observation = observation(&descriptor);
        descriptor.schema_version = 4;
        descriptor.mode = EXACT_ABSENCE_MODE.to_string();
        descriptor.absence_evidence = Some(descriptor.baseline.clone());
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, super::super::ORACLE)
                .is_err()
        );
        assert!(positive_observation.validate(&descriptor).is_ok());
        assert!(observation(&descriptor).validate(&descriptor).is_err());
        descriptor.absence_evidence.as_mut().unwrap().sha256 = AbsenceRule::Exact
            .pins()
            .unwrap()
            .coverage_sha256
            .to_string();
        assert_eq!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, super::super::ORACLE)
                .is_ok(),
            cfg!(target_os = "linux"),
        );
        descriptor.absence_evidence = None;
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, super::super::ORACLE)
                .is_err()
        );
        assert!(AbsenceRule::Substrings.pins().is_ok());
    }

    #[test]
    fn exact_names_preserve_case_and_do_not_include_underscore_or_substring_families() {
        let names =
            PossibleNames::read(br#"["Known", "_possible", "name"]"#, AbsenceRule::Exact).unwrap();
        for value in ["Known", "_possible", "name", "", "a.b", "é"] {
            assert!(
                !names.proves_absence(PythonVersion::Py313, value),
                "{value}"
            );
        }
        for value in ["known", "KNOWN", "_unknown", "Kno", "ame", "999"] {
            assert!(names.proves_absence(PythonVersion::Py313, value), "{value}");
            assert!(!names.proves_absence(PythonVersion::Py312, value));
        }
        for invalid in [
            r#"[]"#,
            r#"[""]"#,
            r#"["a","a"]"#,
            r#"["a","A"]"#,
            r#"["a.b"]"#,
            r#"["é"]"#,
        ] {
            assert!(PossibleNames::read(invalid.as_bytes(), AbsenceRule::Exact).is_err());
        }
    }

    #[test]
    fn exact_coverage_rejects_substring_fields_and_null_policies() {
        let descriptor = partial_descriptor();
        let names = json!({"path": "/evidence/names", "bytes": 1, "sha256": "synthetic names"});
        let value = json!({
            "schema_version": 1, "target": descriptor.target,
            "review_status": "independently-reviewed-source-coverage",
            "polarity": "possible-present-superset", "domain": "nonempty-ascii-alnum-underscore",
            "matching": "case-sensitive-exact", "possible_names": names,
            "source_cookie_policy": "all-byte-offset-normalized-cookie-v1",
            "source_cookie_evidence": {"path": "/evidence/cookie", "bytes": 1, "sha256": COOKIE_EVIDENCE_SHA256},
            "coverage_rows": [{}], "conditions": ["synthetic condition"], "unrestricted_inputs": [{}],
            "required_absent_paths": [REQUIRED_ABSENT_PATH], "artifacts": [names],
        });
        let pins = AbsencePins {
            coverage_sha256: "synthetic aggregate",
            names_sha256: "synthetic names",
        };
        let coverage: AbsenceCoverage = serde_json::from_value(value.clone()).unwrap();
        assert!(
            coverage
                .validate(&descriptor.target, &pins, AbsenceRule::Exact)
                .is_ok()
        );
        assert!(
            coverage
                .validate(&descriptor.target, &pins, AbsenceRule::Substrings)
                .is_err()
        );
        for (key, replacement) in [
            ("always_possible_prefixes", json!(["_"])),
            ("substring_roots", names),
            ("matching", json!("ascii-case-insensitive-substring")),
            (
                "review_status",
                json!("pending-independent-global-precision-review"),
            ),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            let coverage: AbsenceCoverage = serde_json::from_value(changed).unwrap();
            assert!(
                coverage
                    .validate(&descriptor.target, &pins, AbsenceRule::Exact)
                    .is_err(),
                "{key}"
            );
        }
        for key in [
            "possible_names",
            "substring_roots",
            "always_possible_prefixes",
        ] {
            let mut changed = value.clone();
            changed[key] = Value::Null;
            assert!(
                serde_json::from_value::<AbsenceCoverage>(changed).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn cookie_masks_cover_all_raw_offsets_without_decoding() {
        let source = b"first\nsecond\rthird\n\xffcoding:--Foo_-BAR__ coding=.foo. coding:xxcoding:utf8\xff coding:\xffignored coding=\tLATIN-1 coding:___ coding:arbitrarily_long_codec_name_918";
        assert_eq!(
            source_cookie_exceptions(source),
            BTreeSet::from(
                [
                    "foo_bar",
                    "xxcoding",
                    "utf8",
                    "latin_1",
                    "arbitrarily_long_codec_name_918",
                    "utf_8",
                    "iso_8859_1",
                    "utf_8_sig",
                ]
                .map(str::to_string)
            )
        );
    }

    #[test]
    fn same_input_bytes_mask_absence_and_reset_for_the_next_compilation() {
        use super::super::{CompilerInput, compile_module_input};
        use serc::CompileError;

        let marker = "proved_absent_1001";
        let model = LoadedModel {
            state: InitialInternState::from_partial_with_absence(
                std::iter::empty::<&str>(),
                move |version, value| version == PythonVersion::Py313 && value == marker,
            ),
            source_cookie_masks: true,
            required_absent_paths: Vec::new(),
            evidence: Value::Null,
            inputs: Vec::new(),
        };
        let options = CompileOptions {
            python_version: PythonVersion::Py313,
            ..CompileOptions::default()
        };
        let source = "def first(x):\n    return x in {'proved_absent_1001'}\ndef second(x):\n    'different tuple'\n    return x in {'proved_absent_1001'}\n";
        let plain = CompilerInput::Bytes(source.as_bytes().to_vec());
        let masked = CompilerInput::Bytes(
            format!("# line one\n# line two\n# coding:{marker}\n{source}").into_bytes(),
        );
        let compile = |input: &CompilerInput, filename| {
            compile_module_input(input, filename, &options, Some(&model))
        };
        let before = compile(&plain, "absence.py").unwrap().marshal();
        assert_eq!(
            compile(&masked, "absence.py").unwrap_err(),
            CompileError::UnresolvedInternState {
                value: marker.to_string()
            }
        );
        assert_eq!(compile(&plain, "absence.py").unwrap().marshal(), before);
        assert!(compile(&masked, "proved_absent_1001/absence.py").is_ok());
        assert!(matches!(
            compile(&CompilerInput::Text(source.to_string()), "absence.py"),
            Err(CompileError::Internal(_))
        ));
    }

    #[test]
    fn assume_absent_diagnostic_preserves_positive_facts_and_resets_each_filename() {
        use super::super::{CompilerInput, compile_module_input};
        use serc::CompileError;

        let baseline = "diagnostic_baseline_105";
        let phase = "diagnostic_phase_105";
        let unknown = "diagnostic_unknown_105";
        let make_model = |assume_absent| {
            let (state, source_cookie_masks) = initial_state(
                [baseline.to_string()]
                    .into_iter()
                    .chain([phase.to_string()]),
                Some(PossibleNames::Exact(BTreeSet::from([unknown.to_string()]))),
                true,
                assume_absent,
            );
            LoadedModel {
                state,
                source_cookie_masks,
                required_absent_paths: Vec::new(),
                evidence: Value::Null,
                inputs: Vec::new(),
            }
        };
        let diagnostic = make_model(true);
        let proved = make_model(false);
        assert!(!diagnostic.source_cookie_masks);
        assert!(proved.source_cookie_masks);
        let options = CompileOptions {
            python_version: PythonVersion::Py313,
            ..CompileOptions::default()
        };
        for marker in [baseline, phase, unknown] {
            let source = format!(
                "# line one\n# line two\n# coding:{marker}\ndef first(x):\n    return x in {{'{marker}'}}\ndef second(x):\n    'different tuple'\n    return x in {{'{marker}'}}\n"
            );
            let input = CompilerInput::Bytes(source.as_bytes().to_vec());
            let compile = |filename, model: &LoadedModel| {
                compile_module_input(&input, filename, &options, Some(model))
            };
            let bytes = compile("diagnostic.py", &diagnostic).unwrap().marshal();
            let reference = |filename, entries: Vec<&str>| {
                serc::compile_with_options(
                    &source,
                    filename,
                    &CompileOptions {
                        python_version: PythonVersion::Py313,
                        initial_intern_state: InitialInternState::from_complete(entries),
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal()
            };
            assert_eq!(
                bytes,
                reference("diagnostic.py", vec![baseline, phase, "diagnostic.py"])
            );
            if marker == unknown {
                assert_eq!(
                    compile("diagnostic.py", &proved).unwrap_err(),
                    CompileError::UnresolvedInternState {
                        value: unknown.to_string()
                    }
                );
                let filename = "diagnostic_unknown_105/diagnostic.py";
                assert_eq!(
                    compile(filename, &diagnostic).unwrap().marshal(),
                    reference(filename, vec![baseline, phase, unknown, "diagnostic.py"])
                );
                assert_eq!(
                    compile("diagnostic.py", &diagnostic).unwrap().marshal(),
                    bytes
                );
            } else {
                assert_eq!(compile("diagnostic.py", &proved).unwrap().marshal(), bytes);
                assert_ne!(
                    bytes,
                    reference(
                        "diagnostic.py",
                        vec![
                            if marker == baseline { phase } else { baseline },
                            "diagnostic.py"
                        ]
                    )
                );
            }
        }
    }

    #[test]
    fn assume_absent_evidence_labels_the_heuristic_and_retains_provenance() {
        let mut evidence = json!({"mode": EXACT_ABSENCE_MODE, "absence_evidence": {"sha256": "original proof provenance"}});
        mark_assume_absent(&mut evidence);
        assert_eq!(evidence["mode"], EXACT_ABSENCE_MODE);
        assert_eq!(
            evidence["absence_evidence"]["sha256"],
            "original proof provenance"
        );
        assert_eq!(evidence["assumption_status"], "heuristic-not-source-proved");
        assert_eq!(evidence["initial_unknown_assumption"], "absent");
        assert_eq!(evidence["unknown_default"], "assumed-absent");
        assert_eq!(evidence["dependency_policy"], "diagnostic-assume-absent");
        assert_eq!(evidence["source_cookie_unknown_masks"], false);
    }

    #[test]
    fn retained_native_evidence_requires_admission_without_retagging_observations() {
        let mut descriptor = partial_descriptor();
        assert!(descriptor.retained_native_positive_evidence.is_none());
        let prior = observation(&descriptor);
        let mut unadmitted = descriptor.baseline.clone();
        unadmitted.sha256 = "unadmitted".to_string();
        descriptor.retained_native_positive_evidence = Some(unadmitted.clone());
        assert!(
            descriptor
                .validate_contract(PythonVersion::Py313, true, super::super::ORACLE)
                .is_err()
        );
        assert!(prior.validate(&descriptor).is_ok());
        let mut older = self::descriptor();
        older.retained_native_positive_evidence = Some(unadmitted);
        assert!(
            older
                .validate_contract(PythonVersion::Py312, true, super::super::ORACLE)
                .is_err()
        );
    }

    #[test]
    fn retained_native_evidence_binds_target_protocol_and_canonical_names() {
        let descriptor = partial_descriptor();
        let value = json!({
            "schema_version": 1, "kind": "retained-native-positive-v1",
            "target": descriptor.target, "compiler_input_mode": descriptor.compiler_input_mode,
            "oracle_sha256": descriptor.oracle_sha256,
            "sealed_profile_sha256": descriptor.sealed_profile.sha256,
            "executable_sha256": descriptor.executable.sha256,
            "runtime_inputs_sha256": descriptor.runtime_inputs.sha256,
            "names": descriptor.baseline, "artifacts": [descriptor.model.pathlib],
        });
        let check = |value| {
            serde_json::from_value::<RetainedNativePositiveEvidence>(value)
                .unwrap()
                .validate(&descriptor)
        };
        assert!(check(value.clone()).is_ok());
        for (key, replacement) in [
            ("target", json!({})),
            ("compiler_input_mode", json!("text")),
            ("runtime_inputs_sha256", json!("changed")),
            ("artifacts", json!([])),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            assert!(check(changed).is_err(), "{key}");
        }
        assert_eq!(
            read_retained_native_names(br#"["native_owner_106"]"#).unwrap(),
            ["native_owner_106"]
        );
        for invalid in [
            r#"[]"#,
            r#"[""]"#,
            r#"["a","a"]"#,
            r#"["b","a"]"#,
            r#"[null]"#,
        ] {
            assert!(
                read_retained_native_names(invalid.as_bytes()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn retained_native_positives_change_shared_set_identity_in_both_modes() {
        use serc::CompileError;
        let marker = "native_owner_106";
        let source = "def first(x):\n    return x in {'native_owner_106'}\ndef second(x):\n    'different tuple'\n    return x in {'native_owner_106'}\n";
        let compile = |names: Vec<String>, assume_absent| {
            let (state, _) = initial_state(names, None, true, assume_absent);
            serc::compile_with_options(
                source,
                "native.py",
                &CompileOptions {
                    python_version: PythonVersion::Py313,
                    initial_intern_state: state,
                    ..CompileOptions::default()
                },
            )
            .map(|module| module.marshal())
        };
        assert_eq!(
            compile(Vec::new(), false).unwrap_err(),
            CompileError::UnresolvedInternState {
                value: marker.to_string()
            }
        );
        let absent = compile(Vec::new(), true).unwrap();
        let names = read_retained_native_names(br#"["native_owner_106"]"#).unwrap();
        let retained = compile(names.clone(), false).unwrap();
        assert_ne!(retained, absent);
        assert_eq!(compile(names, true).unwrap(), retained);
        assert_eq!(compile(Vec::new(), true).unwrap(), absent);
    }

    #[test]
    fn required_absent_paths_reject_files_and_dangling_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.zip");
        assert!(validate_absent_paths(std::slice::from_ref(&path)).is_ok());
        std::fs::write(&path, b"appeared").unwrap();
        assert!(validate_absent_paths(std::slice::from_ref(&path)).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(directory.path().join("missing"), &path).unwrap();
            assert!(validate_absent_paths(std::slice::from_ref(&path)).is_err());
        }
        assert!(validate_absent_paths(&[PathBuf::from("relative.zip")]).is_err());
    }

    #[test]
    fn pinned_posix_parser_retains_components_but_not_drive_or_root() {
        for (filename, expected, canonical) in [
            ("", vec![], "."),
            (
                "./alpha//alpha/../file.py/",
                vec!["alpha", "alpha", "..", "file.py"],
                "alpha/alpha/../file.py",
            ),
            (
                "//server/share/./file.py",
                vec!["server", "share", "file.py"],
                "//server/share/file.py",
            ),
            (
                "///alpha//file.py",
                vec!["alpha", "file.py"],
                "/alpha/file.py",
            ),
            (
                "C:\\alpha/file.py",
                vec!["C:\\alpha", "file.py"],
                "C:\\alpha/file.py",
            ),
            (
                "../../file.py",
                vec!["..", "..", "file.py"],
                "../../file.py",
            ),
            ("//", vec![], "//"),
        ] {
            assert_eq!(retained_parts(filename).collect::<Vec<_>>(), expected);
            assert_eq!(canonical_filename(filename), canonical);
        }
    }

    #[test]
    fn filename_validation_preserves_the_literal_compiler_filename() {
        for filename in [
            "target/corpus/file.py",
            "//alpha/file.py",
            "../alpha/file.py",
        ] {
            assert!(validate_filename(Path::new(filename)).is_ok());
        }
        for filename in [
            "",
            "./alpha.py",
            "alpha//file.py",
            "alpha/./file.py",
            "///alpha.py",
            "alpha.py/",
        ] {
            assert!(validate_filename(Path::new(filename)).is_err());
        }
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            assert!(validate_filename(Path::new(OsStr::from_bytes(b"bad\xff.py"))).is_err());
        }
    }

    #[test]
    fn filename_states_reset_and_replace_the_neutral_profile() {
        let model = LoadedModel {
            state: InitialInternState::from_complete(["bootstrap_atom"]),
            source_cookie_masks: false,
            required_absent_paths: Vec::new(),
            evidence: Value::Null,
            inputs: Vec::new(),
        };
        let options = CompileOptions {
            python_version: PythonVersion::Py312,
            initial_intern_state: InitialInternState::new(["retained_path_atom"]),
            ..CompileOptions::default()
        };
        let source =
            "f = lambda x: x in {'retained_path_atom', 'z'}\nx in {'retained_path_atom', 'z'}\n";
        let compile = |options: &CompileOptions| {
            serc::compile_with_options(source, "unchanged.py", options)
                .unwrap()
                .marshal()
        };
        let original = compile(&options);
        let baseline = CompileOptions {
            python_version: PythonVersion::Py312,
            initial_intern_state: model.state.clone(),
            ..CompileOptions::default()
        };
        let first = model.options_for("retained_path_atom/file.py", source.as_bytes(), &options);
        let second = model.options_for("other_directory/file.py", source.as_bytes(), &options);
        assert_ne!(compile(&first), compile(&baseline));
        assert_eq!(compile(&second), compile(&baseline));
        assert_eq!(
            compile(&model.options_for("other_directory/file.py", source.as_bytes(), &options)),
            compile(&second),
        );
        assert_eq!(compile(&options), original);
    }

    #[test]
    fn contract_rejects_unreviewed_target_baseline_or_implementation() {
        let oracle = super::super::ORACLE;
        assert_eq!(
            descriptor()
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_ok(),
            cfg!(target_os = "linux")
        );
        for target in [
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert!(
                descriptor()
                    .validate_contract(target, true, oracle)
                    .is_err()
            );
        }
        assert!(
            descriptor()
                .validate_contract(PythonVersion::Py312, false, oracle)
                .is_err()
        );
        assert!(
            descriptor()
                .validate_contract(PythonVersion::Py312, true, "changed oracle")
                .is_err()
        );
        let mut changed = descriptor();
        changed.baseline.sha256 = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
        let mut changed = descriptor();
        changed.model.rust_sha256 = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
        let mut changed = descriptor();
        changed.mode = "unknown".to_string();
        assert!(
            changed
                .validate_contract(PythonVersion::Py312, true, oracle)
                .is_err()
        );
    }
}
