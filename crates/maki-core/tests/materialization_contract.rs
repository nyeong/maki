use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use maki_core::analysis::ProjectSnapshot;
use maki_core::materialization::{
    Association, CurrentInput, DeclaredInput, EffectClass, Fingerprint, Freshness, FreshnessState,
    MaterializationProducer, MaterializedArtifact, OutputKind, PolicyState, ProducerId,
    ReconcileApplyError, ReconcilePlan, ResultPolicy, SourceEdit, SourcePrecondition, SourceRegion,
    SuccessfulProvenance, TransformIdentity, evaluate_materializations,
    executor_config_fingerprint, output_fingerprint,
};
use maki_core::source::SourceSpan;

#[derive(Clone, Copy)]
enum FixtureSetup {
    Direct,
    MissingDependency,
    SelfCycle,
    StaleDependency,
    CycleDependency,
}

struct StateFixture {
    name: &'static str,
    setup: FixtureSetup,
    producer_count: usize,
    artifact_count: usize,
    current_input: Option<u8>,
    current_output: Option<u8>,
    provenance: Option<(u8, u8)>,
    policy: ResultPolicy,
    expected_association: Association,
    expected_direct_freshness: Freshness,
    expected_freshness: Freshness,
    expected_policy: PolicyState,
    expected_primary: FreshnessState,
}

const STATE_FIXTURES: &[StateFixture] = &[
    StateFixture {
        name: "fresh",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Fresh,
        expected_freshness: Freshness::Fresh,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Fresh,
    },
    StateFixture {
        name: "missing",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 0,
        current_input: Some(1),
        current_output: None,
        provenance: None,
        policy: ResultPolicy::Replace,
        expected_association: Association::Missing,
        expected_direct_freshness: Freshness::Blocked,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Missing,
    },
    StateFixture {
        name: "stale-input",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(3),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::StaleInput,
        expected_freshness: Freshness::StaleInput,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::StaleInput,
    },
    // Output integrity outranks simultaneous input drift, a missing dependency,
    // and the frozen policy in the primary projection.
    StateFixture {
        name: "modified-output",
        setup: FixtureSetup::MissingDependency,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(3),
        current_output: Some(4),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Frozen,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::ModifiedOutput,
        expected_freshness: Freshness::ModifiedOutput,
        expected_policy: PolicyState::Frozen,
        expected_primary: FreshnessState::ModifiedOutput,
    },
    StateFixture {
        name: "orphan",
        setup: FixtureSetup::Direct,
        producer_count: 0,
        artifact_count: 1,
        current_input: None,
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Orphan,
        expected_direct_freshness: Freshness::Blocked,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Orphan,
    },
    StateFixture {
        name: "ambiguous-producer",
        setup: FixtureSetup::Direct,
        producer_count: 2,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Ambiguous,
        expected_direct_freshness: Freshness::Blocked,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Ambiguous,
    },
    StateFixture {
        name: "ambiguous-result",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 2,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Frozen,
        expected_association: Association::Ambiguous,
        expected_direct_freshness: Freshness::Blocked,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Frozen,
        expected_primary: FreshnessState::Ambiguous,
    },
    StateFixture {
        name: "blocked-cycle",
        setup: FixtureSetup::SelfCycle,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Frozen,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Fresh,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Frozen,
        expected_primary: FreshnessState::Blocked,
    },
    StateFixture {
        name: "unverifiable",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 1,
        current_input: None,
        current_output: Some(2),
        provenance: None,
        policy: ResultPolicy::Replace,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Unverifiable,
        expected_freshness: Freshness::Unverifiable,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Unverifiable,
    },
    // Frozen is the primary label while the richer axis keeps the drift.
    StateFixture {
        name: "frozen",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(3),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Frozen,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::StaleInput,
        expected_freshness: Freshness::StaleInput,
        expected_policy: PolicyState::Frozen,
        expected_primary: FreshnessState::Frozen,
    },
    // Volatile outranks its necessarily unverifiable direct freshness.
    StateFixture {
        name: "volatile",
        setup: FixtureSetup::Direct,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Volatile,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Unverifiable,
        expected_freshness: Freshness::Unverifiable,
        expected_policy: PolicyState::Volatile,
        expected_primary: FreshnessState::Volatile,
    },
    StateFixture {
        name: "downstream-stale",
        setup: FixtureSetup::StaleDependency,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Fresh,
        expected_freshness: Freshness::StaleInput,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::StaleInput,
    },
    StateFixture {
        name: "downstream-blocked",
        setup: FixtureSetup::CycleDependency,
        producer_count: 1,
        artifact_count: 1,
        current_input: Some(1),
        current_output: Some(2),
        provenance: Some((1, 2)),
        policy: ResultPolicy::Replace,
        expected_association: Association::Matched,
        expected_direct_freshness: Freshness::Fresh,
        expected_freshness: Freshness::Blocked,
        expected_policy: PolicyState::Managed,
        expected_primary: FreshnessState::Blocked,
    },
];

fn fingerprint(value: u8) -> Fingerprint {
    Fingerprint::from_bytes([value; 32])
}

fn producer_id(value: &str) -> ProducerId {
    value.parse().expect("fixture producer ID should be valid")
}

fn transform() -> TransformIdentity {
    TransformIdentity::new(
        "fixture-transform",
        "1",
        "fixture-executor",
        executor_config_fingerprint(b"fixture-config"),
    )
    .expect("fixture transform should be valid")
}

fn region(path: impl Into<PathBuf>, offset: usize) -> SourceRegion {
    SourceRegion::new(path, SourceSpan::new(offset, offset + 1))
}

fn producer(
    id: &str,
    path: impl Into<PathBuf>,
    current_input: Option<u8>,
    policy: ResultPolicy,
    dependencies: &[&str],
) -> MaterializationProducer {
    MaterializationProducer {
        id: producer_id(id),
        region: region(path, 0),
        current_input: current_input.map_or(CurrentInput::Unverifiable, |value| {
            CurrentInput::Available(fingerprint(value))
        }),
        transform: transform(),
        effect: EffectClass::PureDocument,
        output_kind: OutputKind::Text,
        policy,
        declared_inputs: dependencies
            .iter()
            .map(|id| DeclaredInput::producer(producer_id(id)))
            .collect(),
    }
}

fn artifact(
    id: &str,
    path: impl Into<PathBuf>,
    current_output: Option<u8>,
    provenance_fingerprints: Option<(u8, u8)>,
) -> MaterializedArtifact {
    MaterializedArtifact {
        region: region(path, 0),
        generated_from: producer_id(id),
        current_output: current_output.map(fingerprint),
        successful_provenance: provenance_fingerprints.map(|(input, output)| {
            SuccessfulProvenance {
                input: fingerprint(input),
                output: fingerprint(output),
                transform: transform(),
                effect: EffectClass::PureDocument,
            }
        }),
    }
}

fn fresh_pair(id: &str, dependencies: &[&str]) -> (MaterializationProducer, MaterializedArtifact) {
    (
        producer(
            id,
            format!("{id}-producer.maki"),
            Some(1),
            ResultPolicy::Replace,
            dependencies,
        ),
        artifact(id, format!("{id}-artifact.maki"), Some(2), Some((1, 2))),
    )
}

fn arrange(fixture: &StateFixture) -> (Vec<MaterializationProducer>, Vec<MaterializedArtifact>) {
    let mut producers = Vec::new();
    let mut artifacts = Vec::new();
    let target_dependencies: &[&str] = match fixture.setup {
        FixtureSetup::Direct => &[],
        FixtureSetup::MissingDependency => &["absent-dependency"],
        FixtureSetup::SelfCycle => &[fixture.name],
        FixtureSetup::StaleDependency => {
            producers.push(producer(
                "stale-upstream",
                "z-stale-upstream-producer.maki",
                Some(3),
                ResultPolicy::Replace,
                &[],
            ));
            artifacts.push(artifact(
                "stale-upstream",
                "z-stale-upstream-artifact.maki",
                Some(2),
                Some((1, 2)),
            ));
            &["stale-upstream"]
        }
        FixtureSetup::CycleDependency => {
            let (cycle_z, cycle_z_artifact) = fresh_pair("cycle-z", &["cycle-a"]);
            let (cycle_a, cycle_a_artifact) = fresh_pair("cycle-a", &["cycle-z"]);
            producers.extend([cycle_z, cycle_a]);
            artifacts.extend([cycle_z_artifact, cycle_a_artifact]);
            &["cycle-z"]
        }
    };

    for index in (0..fixture.producer_count).rev() {
        producers.push(producer(
            fixture.name,
            format!("{}-producer-{index}.maki", fixture.name),
            fixture.current_input,
            fixture.policy,
            target_dependencies,
        ));
    }
    for index in (0..fixture.artifact_count).rev() {
        artifacts.push(artifact(
            fixture.name,
            format!("{}-artifact-{index}.maki", fixture.name),
            fixture.current_output,
            fixture.provenance,
        ));
    }

    (producers, artifacts)
}

#[test]
fn state_fixtures_lock_primary_labels_precedence_and_dependency_propagation() {
    for fixture in STATE_FIXTURES {
        let (producers, artifacts) = arrange(fixture);
        let report = evaluate_materializations(producers.clone(), artifacts.clone());

        let mut reversed_producers = producers;
        let mut reversed_artifacts = artifacts;
        reversed_producers.reverse();
        reversed_artifacts.reverse();
        let reversed = evaluate_materializations(reversed_producers, reversed_artifacts);
        assert_eq!(report, reversed, "{} input ordering", fixture.name);

        let evaluation = report
            .get(&producer_id(fixture.name))
            .expect("fixture target should be evaluated");
        assert_eq!(
            evaluation.association, fixture.expected_association,
            "{} association",
            fixture.name
        );
        assert_eq!(
            evaluation.direct_freshness, fixture.expected_direct_freshness,
            "{} direct freshness",
            fixture.name
        );
        assert_eq!(
            evaluation.freshness, fixture.expected_freshness,
            "{} propagated freshness",
            fixture.name
        );
        assert_eq!(
            evaluation.policy, fixture.expected_policy,
            "{} policy",
            fixture.name
        );
        assert_eq!(
            evaluation.primary_state, fixture.expected_primary,
            "{} primary state",
            fixture.name
        );
        assert_eq!(
            evaluation.has_duplicate_producers(),
            fixture.producer_count > 1,
            "{} duplicate producer association",
            fixture.name
        );
        assert_eq!(
            evaluation.has_duplicate_artifacts(),
            fixture.artifact_count > 1,
            "{} duplicate artifact association",
            fixture.name
        );
        assert_eq!(
            evaluation.has_orphan_artifact(),
            fixture.producer_count == 0 && fixture.artifact_count > 0,
            "{} orphan association",
            fixture.name
        );
        assert!(
            evaluation
                .producer_regions
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "{} producer region ordering",
            fixture.name
        );
        assert!(
            evaluation
                .artifact_regions
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
            "{} artifact region ordering",
            fixture.name
        );
        assert!(
            report.entries().keys().is_sorted(),
            "{} report key ordering",
            fixture.name
        );
    }

    let mut blocked_input = producer(
        "blocked-input",
        "blocked-input-producer.maki",
        Some(1),
        ResultPolicy::Replace,
        &["declared-upstream"],
    );
    blocked_input.current_input = CurrentInput::Blocked;
    assert_eq!(
        blocked_input
            .dependencies()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["declared-upstream"]
    );
    let blocked_report = evaluate_materializations(
        [blocked_input],
        [artifact(
            "blocked-input",
            "blocked-input-artifact.maki",
            Some(2),
            Some((1, 2)),
        )],
    );
    assert_eq!(
        blocked_report
            .get(&producer_id("blocked-input"))
            .expect("blocked input should be evaluated")
            .direct_freshness,
        Freshness::Blocked
    );

    let observed_labels = STATE_FIXTURES
        .iter()
        .map(|fixture| fixture.expected_primary)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        observed_labels,
        std::collections::BTreeSet::from([
            FreshnessState::Fresh,
            FreshnessState::Missing,
            FreshnessState::StaleInput,
            FreshnessState::ModifiedOutput,
            FreshnessState::Orphan,
            FreshnessState::Ambiguous,
            FreshnessState::Blocked,
            FreshnessState::Unverifiable,
            FreshnessState::Frozen,
            FreshnessState::Volatile,
        ])
    );
}

const FIXED_POINT_PATH: &str = "result.maki";
const STALE_SOURCE: &str = "generated: stale\n";
const DESIRED_SOURCE: &str = "generated: expected\n";

fn snapshot(source: &str) -> ProjectSnapshot {
    ProjectSnapshot::compile(BTreeMap::from([(
        PathBuf::from(FIXED_POINT_PATH),
        source.to_string(),
    )]))
}

fn fake_pure_document_reconcile(snapshot: &ProjectSnapshot) -> ReconcilePlan {
    let path = Path::new(FIXED_POINT_PATH);
    let original = snapshot
        .source(path)
        .expect("fixture source should be present");
    let edit = (original != DESIRED_SOURCE)
        .then(|| SourceEdit::new(path, SourceSpan::new(0, original.len()), DESIRED_SOURCE));

    ReconcilePlan::new(
        snapshot.revision(),
        EffectClass::PureDocument,
        [SourcePrecondition::new(path, original)],
        edit,
    )
    .expect("fake materializer should produce a valid plan")
}

fn fake_materialization_check(snapshot: &ProjectSnapshot) -> FreshnessState {
    let id = "fixed-point";
    let source = snapshot
        .source(Path::new(FIXED_POINT_PATH))
        .expect("fixture source should be present");
    let input = output_fingerprint(b"declared-input");
    let desired_output = output_fingerprint(DESIRED_SOURCE);
    let producer = MaterializationProducer {
        id: producer_id(id),
        region: SourceRegion::new(FIXED_POINT_PATH, SourceSpan::new(0, source.len())),
        current_input: CurrentInput::Available(input),
        transform: transform(),
        effect: EffectClass::PureDocument,
        output_kind: OutputKind::Text,
        policy: ResultPolicy::Replace,
        declared_inputs: Vec::new(),
    };
    let artifact = MaterializedArtifact {
        region: SourceRegion::new(FIXED_POINT_PATH, SourceSpan::new(0, source.len())),
        generated_from: producer_id(id),
        current_output: Some(output_fingerprint(source)),
        successful_provenance: Some(SuccessfulProvenance {
            input,
            output: desired_output,
            transform: transform(),
            effect: EffectClass::PureDocument,
        }),
    };

    evaluate_materializations([producer], [artifact])
        .get(&producer_id(id))
        .expect("fake materialization should be evaluated")
        .primary_state
}

#[test]
fn pure_document_reconcile_reaches_a_checked_fixed_point_and_preserves_failures() {
    let original = snapshot(STALE_SOURCE);
    let original_revision = original.revision();
    assert_eq!(
        fake_materialization_check(&original),
        FreshnessState::ModifiedOutput
    );

    let first_plan = fake_pure_document_reconcile(&original);
    assert!(!first_plan.is_empty());
    let updated = first_plan
        .apply(&original)
        .expect("first plan should apply");
    assert_eq!(
        updated.source(Path::new(FIXED_POINT_PATH)),
        Some(DESIRED_SOURCE)
    );
    assert_eq!(
        fake_materialization_check(&updated),
        FreshnessState::Fresh,
        "materialization_check(update(D)) must be fresh"
    );

    let second_plan = fake_pure_document_reconcile(&updated);
    assert!(second_plan.is_empty());
    let fixed_point = second_plan
        .apply(&updated)
        .expect("empty fixed-point plan should apply");
    assert_eq!(fixed_point, updated);
    assert_eq!(fixed_point.revision(), updated.revision());

    let same_source_new_revision = snapshot(STALE_SOURCE);
    assert!(matches!(
        first_plan.apply(&same_source_new_revision),
        Err(ReconcileApplyError::StaleRevision { .. })
    ));
    assert_eq!(
        same_source_new_revision.source(Path::new(FIXED_POINT_PATH)),
        Some(STALE_SOURCE)
    );

    let invalid_read_set = ReconcilePlan::new(
        original.revision(),
        EffectClass::PureDocument,
        [SourcePrecondition::new(
            FIXED_POINT_PATH,
            "different source",
        )],
        [SourceEdit::new(
            FIXED_POINT_PATH,
            SourceSpan::new(0, 1),
            "G",
        )],
    )
    .expect("read-set fixture should be a structurally valid plan");
    assert!(matches!(
        invalid_read_set.apply(&original),
        Err(ReconcileApplyError::SourceChanged { .. })
    ));
    assert_eq!(original.revision(), original_revision);
    assert_eq!(
        original.source(Path::new(FIXED_POINT_PATH)),
        Some(STALE_SOURCE),
        "failed plans must leave their input snapshot unchanged"
    );
}
