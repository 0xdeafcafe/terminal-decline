use term_pty::{Command, Pty, Size};

#[test]
fn runs_command_and_reads_output() {
    let cmd = Command {
        argv: vec!["sh".into(), "-c".into(), "printf hi; stty size; printf \"$FOO\"".into()],
        cwd: None,
        env: vec![("FOO".into(), "bar".into())],
    };
    let pty = Pty::spawn(
        &cmd,
        Size {
            cols: 77,
            lines: 33,
            cell_width: 8,
            cell_height: 16,
        },
    )
    .unwrap();
    let mut r = pty.reader().unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = r.read_batch(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("hi"), "{s:?}");
    assert!(s.contains("33 77"), "{s:?}");
    assert!(s.ends_with("bar"), "env from a bare program name: {s:?}");
}
