//! Boundary stubs shared by the sample generator and contract tests.

use std::error::Error;

use cortexkit_store::{Isolation, StorageBackend, StorageDescriptor};
use cortexkit_test_support::ScratchDir;
use occipital_lang::{
    CheckResult, Decimal, Diagnostic, DiagnosticCode, DiagnosticPhase, FallbackText, HostError,
    Refused, TrialOutcome, TrialResult, VersionDescriptor,
};
use occipital_module::{
    native::{envelope, Boundary, Executor, Language},
    serve::NativeExecutor,
    show_ui::{prepare, PreparedCall, RouteStamp},
    store,
};
use serde_json::{json, Value};
use subc_client_rs::HandlerOutcome;

pub const SAMPLE_ID: &str = "r_00000000000000000000000000000000";
pub const SOURCE: &str = "Text(\"Hello 雪\")";

pub fn arguments() -> Value {
    json!({"kind":"native", "program":SOURCE, "title":"Title 🦉", "summary":"Summary 雪"})
}

pub fn stamp() -> Result<RouteStamp, Box<dyn Error>> {
    let bind: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/pinned/subc-protocol-0.33.1/module_control_request_route_bind_with_scope.json"
    ))?;
    Ok(RouteStamp {
        scope: Some(serde_json::from_value(bind["scope"].clone())?),
        principal: Some(serde_json::from_value(bind["principal"].clone())?),
    })
}

pub fn call() -> Result<PreparedCall, Box<dyn Error>> {
    prepare(
        &arguments(),
        None,
        &stamp()?,
        1234,
        Decimal::from_integer(1234).map_err(|e| e.code())?,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message).into())
}

pub fn descriptor(dir: &ScratchDir) -> StorageDescriptor {
    StorageDescriptor {
        module_id: "occipital".into(),
        storage_namespace: "default".into(),
        isolation: Isolation::Module,
        backend: StorageBackend::Sqlite {
            path: dir.path().join("store.db").to_string_lossy().into_owned(),
        },
    }
}

pub fn trial(outcome: TrialOutcome) -> TrialResult {
    TrialResult {
        fuel: 17,
        nodes: 1,
        outcome,
    }
}

pub fn diagnostic(warning: bool, name: &str, padding: usize) -> Diagnostic {
    Diagnostic::new(
        if warning {
            DiagnosticCode::Unused
        } else {
            DiagnosticCode::Type
        },
        DiagnosticPhase::Check,
    )
    .with_subject(&format!("{name}{}", "雪".repeat(padding)))
}

pub struct Stub(pub &'static str);

impl Stub {
    pub fn result(&self) -> CheckResult {
        let (diagnostics, outcome) = match self.0 {
            "abort" => (vec![], TrialOutcome::Fuel),
            "error" => (
                vec![diagnostic(false, "first-error", 0)],
                TrialOutcome::NotRun,
            ),
            "trim_errors" => (
                vec![
                    diagnostic(false, "first-error", 0),
                    diagnostic(true, "first-warning", 0),
                    diagnostic(false, "middle-error", 3500),
                    diagnostic(true, "last-warning", 6000),
                    diagnostic(false, "last-error", 6000),
                ],
                TrialOutcome::NotRun,
            ),
            "trim_warnings" => (
                vec![
                    diagnostic(true, "first-warning", 0),
                    diagnostic(true, "middle-warning", 3500),
                    diagnostic(true, "last-warning", 6000),
                ],
                TrialOutcome::Ok,
            ),
            _ => (vec![], TrialOutcome::Ok),
        };
        CheckResult::new(diagnostics, trial(outcome))
    }
}

impl Boundary for Stub {
    fn check(&self, _call: &PreparedCall) -> Result<CheckResult, HostError> {
        match self.0 {
            "rejected" => Err(HostError::Rejected(vec![])),
            "refused" => Err(HostError::Refused(Refused::UnitFuel)),
            "invalid_host" => Err(HostError::InvalidHostValue("stub value")),
            "internal" => Err(HostError::Internal("stub failure")),
            "panic_check" => panic!("stub check panic"),
            _ => Ok(self.result()),
        }
    }

    fn fallback(&self, call: &PreparedCall) -> FallbackText {
        if self.0 == "panic_fallback" {
            panic!("stub fallback panic");
        }
        if self.0 == "metadata" {
            return FallbackText {
                text: occipital_lang::DisplayText::new("Fallback 雪"),
                fuel: 3,
                scanned_bytes: 4,
                truncated: true,
            };
        }
        Language.fallback(call)
    }

    fn version(&self) -> VersionDescriptor {
        if self.0 == "panic_version" {
            panic!("stub version panic");
        }
        let mut version = VersionDescriptor::current();
        if self.0 == "metadata" {
            version.execution_dialect = "9.8".into();
            version.catalog.version = "7.6".into();
            version.catalog.digest = "ab".repeat(32);
            version.catalog.layout = "5".into();
            version.catalog.aliases = vec!["cd".repeat(32)];
        }
        if matches!(self.0, "cap_at" | "cap_over") {
            version.catalog.version.clear();
            let content = json!({
                "schema":"occipital.show_ui/1", "outcome":"stored", "kind":"native",
                "recordId":SAMPLE_ID, "check":self.result(), "version":version
            });
            let base = envelope(content, None)
                .expect("sample envelope serializes")
                .len();
            let target = if self.0 == "cap_at" { 32_768 } else { 32_769 };
            let padding = target - base;
            version.catalog.version =
                format!("{}{}", "雪".repeat(padding / 3), "x".repeat(padding % 3));
        }
        version
    }
}

pub fn decode(outcome: HandlerOutcome) -> Result<(Value, usize), Box<dyn Error>> {
    let HandlerOutcome::Response(bytes) = outcome else {
        return Err(format!("expected tool result, got {outcome:?}").into());
    };
    Ok((serde_json::from_slice(&bytes)?, bytes.len()))
}

pub fn normalize(mut value: Value) -> Value {
    if value["structuredContent"]["recordId"].is_string() {
        value["structuredContent"]["recordId"] = SAMPLE_ID.into();
    }
    value
}

pub fn samples() -> Result<Value, Box<dyn Error>> {
    let dir = ScratchDir::new("occipital-samples");
    let db = store::open(&descriptor(&dir)).map_err(|e| format!("{e:?}"))?;
    let mut samples = vec![];
    for name in [
        "stored",
        "check_failed",
        "abort",
        "error",
        "rejected",
        "refused",
        "invalid_host",
        "internal",
        "cap_at",
        "cap_over",
        "trim_errors",
        "trim_warnings",
        "store_failed",
    ] {
        let mut prepared = call()?;
        let mut arguments = arguments();
        if name == "check_failed" {
            prepared.source = "{Text(}".into();
            arguments["program"] = prepared.source.clone().into();
        }
        let output = match name {
            "stored" | "check_failed" => Executor(Language).execute(prepared, Some(&db)),
            "store_failed" => Executor(Stub("ok")).execute(prepared, None),
            _ => Executor(Stub(name)).execute(prepared, Some(&db)),
        };
        let (output, bytes) = decode(output)?;
        samples.push(json!({"name":name, "arguments":arguments, "response":normalize(output), "envelopeBytes":bytes}));
        if name == "cap_over" {
            let candidate = json!({"schema":"occipital.show_ui/1", "outcome":"stored", "kind":"native",
                "recordId":SAMPLE_ID, "check":Stub(name).result(), "version":Stub(name).version()});
            samples.push(json!({"name":"oversize_candidate", "envelopeBytes":envelope(candidate.clone(), None)?.len(),
                "candidate":serde_json::from_slice::<Value>(&envelope(candidate, None)?)?}));
        }
    }
    let refusals: Value =
        serde_json::from_str(include_str!("../../test-vectors/show-ui-v1/refusals.json"))?;
    for case in refusals
        .as_array()
        .ok_or("refusal cases must be an array")?
    {
        let route = if case["scoped"] == true {
            stamp()?
        } else {
            RouteStamp::default()
        };
        let raw = case
            .get("output_refs")
            .map(serde_json::to_vec)
            .transpose()?;
        let refusal = match prepare(
            &case["arguments"],
            raw.as_deref(),
            &route,
            1234,
            Decimal::from_integer(1234).map_err(|e| e.code())?,
        ) {
            Ok(_) => return Err("expected sample refusal".into()),
            Err(refusal) => refusal,
        };
        let (response, _) = decode(occipital_module::native::refused(refusal))?;
        samples.push(json!({"name":case["name"], "arguments":case["arguments"], "output_refs":case.get("output_refs"), "response":response}));
    }
    Ok(Value::Array(samples))
}
