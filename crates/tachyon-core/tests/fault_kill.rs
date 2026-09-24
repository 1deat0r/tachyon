//! M12 armed fault-point kill: child parks at `evidence.read`, parent
//! SIGKILLs it, parent continues (the process-death half of ADR 0001).

use std::process::Command;
use std::time::Duration;

const ARMED: &str = "TACHYON_M12_FAULT_KILL_CHILD";
const SEAM: &str = "evidence.read";

fn armed_child() {
    // Parent set TACHYON_FAULT_POINT=evidence.read; park until killed.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        tachyon_tools::fault::reach(SEAM).await;
    });
    // Unreachable when armed; keep a clean exit if env was lost.
    std::process::exit(0);
}

#[test]
fn fault_kill_child_parks_at_evidence_read_and_is_killed() {
    if std::env::var(ARMED).is_ok() {
        armed_child();
        return;
    }

    let exe = std::env::current_exe().expect("current exe");
    let mut child = Command::new(&exe)
        .args([
            "--exact",
            "fault_kill_child_parks_at_evidence_read_and_is_killed",
            "--nocapture",
        ])
        .env(ARMED, "1")
        .env("TACHYON_FAULT_POINT", SEAM)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn armed child");

    // Give the child time to reach the seam and park.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "child should still be parked at the armed fault point"
    );

    child.kill().expect("SIGKILL armed child");
    let status = child.wait().expect("wait dead child");
    assert!(
        !status.success(),
        "killed child must not exit 0: {status:?}"
    );
}
