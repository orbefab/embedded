use serde::Serialize;
use sim_core::name::Ref;
use sim_core::netlist;
use sim_core::run_folder;
use sim_core::timestamp::Timestamp;

use crate::Status;

const DIVIDER: &str = include_str!("../../../examples/divider/netlist.json");

#[derive(Serialize)]
pub struct DoctorReport {
    pub engine_version: &'static str,
    pub netlist_versions: &'static [u64],
    pub run_format: u64,
    pub checks: Vec<Check>,
}

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl DoctorReport {
    pub fn status(&self) -> Status {
        if self.checks.iter().all(|c| c.ok) {
            Status::Ok
        } else {
            Status::Failed
        }
    }
}

/// Checks, in order:
/// - `self_test.divider`: no diagnostics, and `#net.vout` within 1e-8 relative of 10/3.
/// - `determinism`: two renders of that netlist at one timestamp are byte-identical.
pub fn doctor() -> DoctorReport {
    let at = Timestamp::from_unix_seconds(0);
    DoctorReport {
        engine_version: run_folder::ENGINE_VERSION,
        netlist_versions: netlist::VERSIONS,
        run_format: run_folder::FORMAT_VERSION,
        checks: vec![divider_check(at), determinism_check(at)],
    }
}

fn divider_check(at: Timestamp) -> Check {
    let name = "self_test.divider";
    let done = crate::run(DIVIDER.as_bytes(), at);
    let expect = 10.0 / 3.0;
    let vout_ref: Ref = "#net.vout".parse().expect("#net.vout is a valid ref");
    let Some(vout) = done
        .results
        .observations()
        .op()
        .and_then(|op| op.values.get(&vout_ref))
    else {
        return Check {
            name,
            ok: false,
            detail: "no #net.vout".to_string(),
        };
    };
    let got = vout.get();
    let close = (got - expect).abs() <= 1e-8 * expect.abs();
    let ok = done.results.diagnostics().is_empty() && close;
    let got_json = serde_json::to_string(vout).expect("a finite number serializes");
    let expect_json = serde_json::to_string(&expect).expect("a finite number serializes");
    let detail = if done.results.diagnostics().is_empty() {
        format!("#net.vout = {got_json} V, closed form {expect_json} V")
    } else {
        let codes: Vec<_> = done
            .results
            .diagnostics()
            .iter()
            .map(|d| d.code())
            .collect();
        format!("diagnostics {}", codes.join(", "))
    };
    Check { name, ok, detail }
}

fn determinism_check(at: Timestamp) -> Check {
    let name = "determinism";
    let left = crate::run(DIVIDER.as_bytes(), at);
    let right = crate::run(DIVIDER.as_bytes(), at);
    let files = left.folder.files().count();
    let ok = left.folder == right.folder && files == 4;
    let detail = if ok {
        "two renders byte-identical (4 files)".to_string()
    } else {
        format!("two renders differ ({files} files)")
    };
    Check { name, ok, detail }
}
