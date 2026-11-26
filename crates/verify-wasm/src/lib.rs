//! WASM bindings used by `web/src/verifier.ts`.

use wasm_bindgen::prelude::*;

/// Decode and render a raw journal. Does NOT verify anything.
#[wasm_bindgen]
pub fn inspect_journal(journal: &[u8]) -> Result<String, JsError> {
    let j = margin_journal::Journal::decode(journal).map_err(|e| JsError::new(&e.to_string()))?;
    margin_verify::render_journal(&j).map_err(|e| JsError::new(&e.to_string()))
}

/// Verify a receipt, and optionally an authorship claim (the JSON written by `margin claim`).
/// Returns JSON `{ "valid": bool, "report": string, "authorship": null | { "ok": bool, "message": string } }`.
/// `authorship` is only set when a claim was given and the receipt is valid.
#[wasm_bindgen]
pub fn verify_receipt(receipt: &[u8], checkers_json: &str, envs_json: &str, claim_json: Option<String>) -> String {
    let (valid, report, authorship) = verify(receipt, checkers_json, envs_json, claim_json.as_deref());
    let authorship = authorship.map(|(ok, message)| serde_json::json!({ "ok": ok, "message": message }));
    serde_json::json!({ "valid": valid, "report": report, "authorship": authorship }).to_string()
}

type Outcome = (bool, String, Option<(bool, String)>);

#[cfg(feature = "risc0")]
fn verify(receipt: &[u8], checkers_json: &str, envs_json: &str, claim_json: Option<&str>) -> Outcome {
    let accepted = match margin_verify::load_checkers(checkers_json) {
        Ok(c) => c,
        Err(e) => return (false, format!("bad checkers.json: {e}"), None),
    };
    let envs = match margin_verify::load_environments(envs_json) {
        Ok(e) => e,
        Err(e) => return (false, format!("bad envs.json: {e}"), None),
    };
    if accepted.is_empty() {
        return (false, "No accepted checkers are published yet.".into(), None);
    }
    let v = match margin_verify::verify_receipt(receipt, &accepted, &envs) {
        Ok(v) => v,
        Err(e) => return (false, e.to_string(), None),
    };
    let report = match margin_verify::render_journal(&v.journal) {
        Ok(r) => format!("accepted checker:     {} ({})\naccepted environment: {}\n\n{r}", v.checker.name, v.checker.status, v.environment.name),
        Err(e) => return (false, format!("receipt verified but journal is malformed: {e}"), None),
    };
    let authorship = claim_json.map(|c| {
        let claim: margin_verify::authorship::Authorship = match serde_json::from_str(c) {
            Ok(a) => a,
            Err(e) => return (false, format!("not an authorship claim: {e}")),
        };
        match margin_verify::authorship::verify(&v.journal, &claim) {
            Ok(()) => (
                true,
                format!(
                    "Signed by the owner key {}. Note: {:?}",
                    hex::encode(v.journal.owner),
                    claim.note
                ),
            ),
            Err(e) => (false, e.to_string()),
        }
    });
    (true, report, authorship)
}

#[cfg(not(feature = "risc0"))]
fn verify(_: &[u8], _: &str, _: &str, _: Option<&str>) -> Outcome {
    (false, "built without receipt verification".into(), None)
}
