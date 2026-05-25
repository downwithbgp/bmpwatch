use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Write;
use std::time::Duration;

fn wait_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn send_key(writer: &mut Box<dyn std::io::Write + Send>, key: u8) {
    let _ = writer.write(&[key]);
    let _ = writer.flush();
}

fn spawn(
    mock_args: &[&str],
) -> (
    Box<dyn portable_pty::MasterPty>,
    Box<dyn std::io::Write + Send>,
) {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new("cargo");
    let mut args = vec!["run", "--bin", "bmpwatch", "--"];
    args.extend_from_slice(mock_args);
    cmd.args(&args);
    let _child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let writer = pair.master.take_writer().expect("take_writer");
    (pair.master, writer)
}

fn enter_waiting(writer: &mut Box<dyn std::io::Write + Send>) {
    // Let app start and render browser
    wait_ms(2000);
    // Select first collector (Enter)
    send_key(writer, 0x0d);
    wait_ms(500);
    // Select first stream (Enter) → enters loading/waiting state
    send_key(writer, 0x0d);
    wait_ms(2000);
}

// ── Waiting-state tests ──

#[test]
fn test_mock_waiting_q_returns_to_browser() {
    let (_master, mut writer) = spawn(&["--mock"]);
    enter_waiting(&mut writer);
    // q in waiting → back to browser
    send_key(&mut writer, b'q');
    wait_ms(1500);
    // Esc to exit browser cleanly
    send_key(&mut writer, 0x1b);
    wait_ms(1000);
}

#[test]
fn test_mock_waiting_b_returns_to_browser() {
    let (_master, mut writer) = spawn(&["--mock"]);
    enter_waiting(&mut writer);
    send_key(&mut writer, b'b');
    wait_ms(1500);
    send_key(&mut writer, 0x1b);
    wait_ms(1000);
}

#[test]
fn test_mock_waiting_esc_exits() {
    let (_master, mut writer) = spawn(&["--mock"]);
    enter_waiting(&mut writer);
    send_key(&mut writer, 0x1b);
    wait_ms(1500);
}

// ── Active-stream tests (--mock --mock-active) ──

fn enter_active(writer: &mut Box<dyn std::io::Write + Send>) {
    // Same as enter_waiting, but the dashboard has a synthetic PeerUp
    enter_waiting(writer);
}

#[test]
fn test_mock_active_q_returns_to_browser() {
    let (_master, mut writer) = spawn(&["--mock", "--mock-active"]);
    enter_active(&mut writer);
    // q in active stream → back to browser (process stays alive)
    send_key(&mut writer, b'q');
    wait_ms(1500);
    // Esc to exit browser cleanly
    send_key(&mut writer, 0x1b);
    wait_ms(1000);
}

#[test]
fn test_mock_active_b_returns_to_browser() {
    let (_master, mut writer) = spawn(&["--mock", "--mock-active"]);
    enter_active(&mut writer);
    send_key(&mut writer, b'b');
    wait_ms(1500);
    send_key(&mut writer, 0x1b);
    wait_ms(1000);
}

#[test]
fn test_mock_active_esc_exits() {
    let (_master, mut writer) = spawn(&["--mock", "--mock-active"]);
    enter_active(&mut writer);
    send_key(&mut writer, 0x1b);
    wait_ms(1500);
}

// ── Browser lobby: q exits ──

#[test]
fn test_browser_lobby_q_exits() {
    let (_master, mut writer) = spawn(&["--mock"]);
    // Let browser render
    wait_ms(2000);
    // q in browser → exits app
    send_key(&mut writer, b'q');
    wait_ms(1500);
}
