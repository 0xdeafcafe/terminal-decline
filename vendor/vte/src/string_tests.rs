//! Tests for APC strings, DCS `put_slice` and the OSC separators and size
//! policy.

use super::*;

#[derive(Debug, PartialEq, Eq, Clone)]
enum Ev {
    Print(char),
    Execute(u8),
    Esc(u8),
    Csi(char),
    Hook(char),
    /// DCS data; neighbouring pieces are merged, however they were split.
    Dcs(Vec<u8>),
    Unhook,
    /// `dcs_end`, recorded only when `Rec::ends` is set.
    DcsEnd(bool),
    ApcStart,
    /// APC data, merged like `Dcs`.
    Apc(Vec<u8>),
    ApcEnd(bool),
    Osc(Vec<Vec<u8>>, bool),
    OscLarge(Vec<u8>),
    /// Streamed OSC data, merged like `Dcs`.
    OscData(Vec<u8>),
    OscEnd(bool),
}

/// Records everything. `bulk` picks whether DCS data arrives through
/// `put_slice` (overridden) or `put` (through the default `put_slice`).
struct Rec {
    log: Vec<Ev>,
    bulk: bool,
    policy: OscLarge,
    /// Record `dcs_end` itself instead of letting the default call `unhook`.
    ends: bool,
    /// How many separate calls delivered APC / DCS data.
    apc_calls: usize,
    dcs_calls: usize,
}

impl Rec {
    fn new(bulk: bool, policy: OscLarge) -> Rec {
        Rec { log: Vec::new(), bulk, policy, ends: false, apc_calls: 0, dcs_calls: 0 }
    }

    fn data(&mut self, bytes: &[u8], wrap: fn(Vec<u8>) -> Ev) {
        let new = wrap(bytes.to_vec());
        match (self.log.last_mut(), new) {
            (Some(Ev::Dcs(v)), Ev::Dcs(n))
            | (Some(Ev::Apc(v)), Ev::Apc(n))
            | (Some(Ev::OscData(v)), Ev::OscData(n)) => v.extend(n),
            (_, new) => self.log.push(new),
        }
    }
}

impl Perform for Rec {
    fn print(&mut self, c: char) {
        self.log.push(Ev::Print(c));
    }
    fn execute(&mut self, b: u8) {
        self.log.push(Ev::Execute(b));
    }
    fn esc_dispatch(&mut self, _: &[u8], _: bool, b: u8) {
        self.log.push(Ev::Esc(b));
    }
    fn csi_dispatch(&mut self, _: &Params, _: &[u8], _: bool, c: char) {
        self.log.push(Ev::Csi(c));
    }
    fn hook(&mut self, _: &Params, _: &[u8], _: bool, c: char) {
        self.log.push(Ev::Hook(c));
    }
    fn put(&mut self, b: u8) {
        if !self.bulk {
            self.dcs_calls += 1;
            self.data(&[b], Ev::Dcs);
        }
    }
    fn put_slice(&mut self, data: &[u8]) {
        if self.bulk {
            self.dcs_calls += 1;
            self.data(data, Ev::Dcs);
        } else {
            for &b in data {
                self.put(b);
            }
        }
    }
    fn unhook(&mut self) {
        self.log.push(Ev::Unhook);
    }
    fn dcs_end(&mut self, complete: bool) {
        if self.ends {
            self.log.push(Ev::DcsEnd(complete));
        } else {
            self.unhook();
        }
    }
    fn apc_start(&mut self) {
        self.log.push(Ev::ApcStart);
    }
    fn apc_put(&mut self, data: &[u8]) {
        self.apc_calls += 1;
        self.data(data, Ev::Apc);
    }
    fn apc_end(&mut self, complete: bool) {
        self.log.push(Ev::ApcEnd(complete));
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], bell: bool) {
        self.log.push(Ev::Osc(params.iter().map(|p| p.to_vec()).collect(), bell));
    }
    fn osc_large(&mut self, prefix: &[u8]) -> OscLarge {
        self.log.push(Ev::OscLarge(prefix.to_vec()));
        self.policy
    }
    fn osc_put(&mut self, data: &[u8]) {
        self.data(data, Ev::OscData);
    }
    fn osc_end(&mut self, complete: bool) {
        self.log.push(Ev::OscEnd(complete));
    }
}

fn run(input: &[u8]) -> Vec<Ev> {
    run_with(input, OscLarge::Buffer(64 * 1024))
}

fn run_with(input: &[u8], policy: OscLarge) -> Vec<Ev> {
    let mut rec = Rec::new(true, policy);
    Parser::new().advance(&mut rec, input);
    rec.log
}

/// Feed `input` in pieces of `chunk` bytes.
fn run_chunked(input: &[u8], chunk: usize, bulk: bool, policy: OscLarge) -> Vec<Ev> {
    let mut rec = Rec::new(bulk, policy);
    let mut parser = Parser::new();
    for piece in input.chunks(chunk) {
        parser.advance(&mut rec, piece);
    }
    rec.log
}

fn osc(parts: &[&str], bell: bool) -> Ev {
    Ev::Osc(parts.iter().map(|p| p.as_bytes().to_vec()).collect(), bell)
}

#[test]
fn apc_split_across_advance_calls() {
    let input = b"a\x1b_Gi=1,a=T;AAAA\x1b\\b";
    let want = vec![
        Ev::Print('a'),
        Ev::ApcStart,
        Ev::Apc(b"Gi=1,a=T;AAAA".to_vec()),
        Ev::ApcEnd(true),
        Ev::Print('b'),
    ];
    assert_eq!(run(input), want);
    // A split at every position, including between ESC and `\`.
    for at in 0..=input.len() {
        let mut rec = Rec::new(true, OscLarge::Drop);
        let mut parser = Parser::new();
        parser.advance(&mut rec, &input[..at]);
        parser.advance(&mut rec, &input[at..]);
        assert_eq!(rec.log, want, "split at {at}");
    }
    for chunk in 1..6 {
        assert_eq!(run_chunked(input, chunk, true, OscLarge::Drop), want, "chunk {chunk}");
    }
}

#[test]
fn apc_empty_and_with_other_controls() {
    assert_eq!(run(b"\x1b_\x1b\\"), vec![Ev::ApcStart, Ev::ApcEnd(true)]);
    // BEL and other C0 controls other than ESC, CAN and SUB are data.
    assert_eq!(
        run(b"\x1b_G\x07\x00\x7f\xff\x1b\\"),
        vec![Ev::ApcStart, Ev::Apc(b"G\x07\x00\x7f\xff".to_vec()), Ev::ApcEnd(true)]
    );
}

#[test]
fn apc_abandoned() {
    let abandoned = |end: u8| {
        vec![
            Ev::ApcStart,
            Ev::Apc(b"Gab".to_vec()),
            Ev::ApcEnd(false),
            Ev::Execute(end),
            Ev::Print('c'),
        ]
    };
    assert_eq!(run(b"\x1b_Gab\x18c"), abandoned(0x18));
    assert_eq!(run(b"\x1b_Gab\x1ac"), abandoned(0x1a));
    // ESC and anything else: the APC is abandoned and the escape sequence
    // goes on.
    assert_eq!(
        run(b"\x1b_Gab\x1b[Ac"),
        vec![
            Ev::ApcStart,
            Ev::Apc(b"Gab".to_vec()),
            Ev::ApcEnd(false),
            Ev::Csi('A'),
            Ev::Print('c')
        ]
    );
    // ESC ESC \ : the second ESC starts a new escape, which `\` ends.
    assert_eq!(
        run(b"\x1b_G\x1b\x1b\\"),
        vec![Ev::ApcStart, Ev::Apc(b"G".to_vec()), Ev::ApcEnd(false), Ev::Esc(b'\\')]
    );
}

#[test]
fn sos_and_pm_are_still_ignored() {
    assert_eq!(run(b"\x1bXabc\x1b\\d"), vec![Ev::Esc(b'\\'), Ev::Print('d')]);
    assert_eq!(run(b"\x1b^abc\x1b\\d"), vec![Ev::Esc(b'\\'), Ev::Print('d')]);
}

#[test]
fn big_apc_costs_few_calls() {
    let mut input = b"\x1b_Ga=T;".to_vec();
    input.extend(std::iter::repeat(b'A').take(3_000_000));
    input.extend_from_slice(b"\x1b\\");
    let mut rec = Rec::new(true, OscLarge::Drop);
    Parser::new().advance(&mut rec, &input);
    assert_eq!(rec.apc_calls, 1);
    assert_eq!(rec.log.len(), 3);
    match &rec.log[1] {
        Ev::Apc(data) => assert_eq!(data.len(), 3_000_005),
        other => panic!("{other:?}"),
    }
}

#[test]
fn dcs_put_slice_matches_per_byte_put() {
    let input = b"x\x1bP0;1q\"1;1;8;8#0;2;0;0;0~~@@\n\x07\x7f\xc3-$\x1b\\y";
    let bulk = run_chunked(input, input.len(), true, OscLarge::Drop);
    let single = run_chunked(input, input.len(), false, OscLarge::Drop);
    assert_eq!(bulk, single);
    assert_eq!(bulk[0], Ev::Print('x'));
    assert_eq!(bulk[1], Ev::Hook('q'));
    // DEL and the byte >= 0x80 are dropped as before; the rest is data.
    assert_eq!(bulk[2], Ev::Dcs(b"\"1;1;8;8#0;2;0;0;0~~@@\n\x07-$".to_vec()));
    // The ST is consumed, as for APC: no `esc_dispatch` for its `\\`.
    assert_eq!(&bulk[3..], &[Ev::Unhook, Ev::Print('y')][..]);
}

#[test]
fn dcs_put_slice_is_called_with_runs() {
    let mut input = b"\x1bPq".to_vec();
    input.extend(std::iter::repeat(b'~').take(100_000));
    input.extend_from_slice(b"\x1b\\");
    let mut rec = Rec::new(true, OscLarge::Drop);
    Parser::new().advance(&mut rec, &input);
    assert_eq!(rec.dcs_calls, 1);

    // The default `put_slice` calls `put` once per byte, in order.
    let mut rec = Rec::new(false, OscLarge::Drop);
    Parser::new().advance(&mut rec, &input);
    assert_eq!(rec.dcs_calls, 100_000);
}

#[test]
fn dcs_can_aborts() {
    assert_eq!(
        run(b"\x1bPqab\x18c"),
        vec![Ev::Hook('q'), Ev::Dcs(b"ab".to_vec()), Ev::Unhook, Ev::Execute(0x18), Ev::Print('c')]
    );
}

/// Feed `input` in pieces of `chunk` bytes, recording `dcs_end`.
fn run_ends(input: &[u8], chunk: usize) -> Vec<Ev> {
    let mut rec = Rec::new(true, OscLarge::Drop);
    rec.ends = true;
    let mut parser = Parser::new();
    for piece in input.chunks(chunk) {
        parser.advance(&mut rec, piece);
    }
    rec.log
}

#[test]
fn dcs_end_reports_how_the_string_ended() {
    let data = || Ev::Dcs(b"ab".to_vec());
    let cases: &[(&[u8], Vec<Ev>)] = &[
        (b"\x1bPqab\x1b\\c", vec![Ev::Hook('q'), data(), Ev::DcsEnd(true), Ev::Print('c')]),
        (b"\x1bPqab\x9cc", vec![Ev::Hook('q'), data(), Ev::DcsEnd(true), Ev::Print('c')]),
        (
            b"\x1bPqab\x18c",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Execute(0x18), Ev::Print('c')],
        ),
        (
            b"\x1bPqab\x1ac",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Execute(0x1a), Ev::Print('c')],
        ),
        // ESC and anything but `\`: abandoned, and the ESC starts a sequence.
        (
            b"\x1bPqab\x1b[Hc",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Csi('H'), Ev::Print('c')],
        ),
        (
            b"\x1bPqab\x1b7c",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Esc(b'7'), Ev::Print('c')],
        ),
        (
            b"\x1bPqab\x1b\x1b\\c",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Esc(b'\\'), Ev::Print('c')],
        ),
        (
            b"\x1bPqab\x1bPqab\x1b\\",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Hook('q'), data(), Ev::DcsEnd(true)],
        ),
        // CAN right after the ESC: abandoned, then executed as usual.
        (
            b"\x1bPqab\x1b\x18c",
            vec![Ev::Hook('q'), data(), Ev::DcsEnd(false), Ev::Execute(0x18), Ev::Print('c')],
        ),
    ];
    for (input, want) in cases {
        for chunk in [1, 2, 3, input.len()] {
            assert_eq!(&run_ends(input, chunk), want, "{:?}, chunk {chunk}", input.escape_ascii());
        }
    }
}

#[test]
fn dcs_end_waits_for_the_byte_after_esc() {
    let mut rec = Rec::new(true, OscLarge::Drop);
    rec.ends = true;
    let mut parser = Parser::new();
    parser.advance(&mut rec, b"\x1bPqab\x1b");
    assert_eq!(rec.log, vec![Ev::Hook('q'), Ev::Dcs(b"ab".to_vec())]);
    parser.advance(&mut rec, b"\\");
    assert_eq!(rec.log.last(), Some(&Ev::DcsEnd(true)));
}

#[test]
fn dcs_end_defaults_to_unhook() {
    // `Rec::ends` off: the default `dcs_end` calls `unhook`, whatever the end.
    for input in [&b"\x1bPqab\x1b\\"[..], b"\x1bPqab\x18", b"\x1bPqab\x1b[H"] {
        let ev = run(input);
        assert_eq!(ev.iter().filter(|e| **e == Ev::Unhook).count(), 1, "{:?}", input.escape_ascii());
    }
}

#[test]
fn osc_keeps_semicolons_in_a_uri() {
    // OSC 8 with ';' inside the URI: the parts rejoin with ';'.
    let ev = run(b"\x1b]8;id=1;http://a/b;c=d;e\x07");
    assert_eq!(ev, vec![osc(&["8", "id=1", "http://a/b", "c=d", "e"], true)]);
    match &ev[0] {
        Ev::Osc(parts, _) => {
            assert_eq!(parts[2..].join(&b';'), b"http://a/b;c=d;e".to_vec());
        },
        _ => unreachable!(),
    }
    // Empty parts survive.
    assert_eq!(run(b"\x1b]1337;;a;;\x1b\\")[0], osc(&["1337", "", "a", "", ""], false));
    assert_eq!(run(b"\x1b]0;\x07"), vec![osc(&["0", ""], true)]);
}

#[test]
fn osc_part_limit_is_unchanged() {
    // 20 parts: 16 are reported; the rest is cut off, as before.
    let parts: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
    let input = format!("\x1b]{}\x07", parts.join(";"));
    match &run(input.as_bytes())[0] {
        Ev::Osc(got, true) => {
            assert_eq!(got.len(), 16);
            for (i, part) in got.iter().enumerate() {
                assert_eq!(part, format!("p{i}").as_bytes());
            }
        },
        other => panic!("{other:?}"),
    }
    // Exactly 16 parts is reported the same way.
    let input = format!("\x1b]{}\x07", parts[..16].join(";"));
    match &run(input.as_bytes())[0] {
        Ev::Osc(got, true) => assert_eq!(got.len(), 16),
        other => panic!("{other:?}"),
    }
}

/// `ESC ]`, then a string of exactly `len` bytes (it starts `1337;File=` and
/// has a ';' inside), then `end`.
fn long_osc(len: usize, end: &[u8]) -> Vec<u8> {
    let mut v = b"\x1b]1337;File=".to_vec();
    while v.len() - 2 < len {
        v.push(if matches!(v.len(), 500 | 2500) { b';' } else { b'A' + (v.len() % 26) as u8 });
    }
    v.extend_from_slice(end);
    v
}

#[test]
fn osc_up_to_4k_never_asks() {
    let ev = run_with(&long_osc(4096, b"\x07"), OscLarge::Drop);
    assert_eq!(ev.len(), 1);
    assert!(matches!(&ev[0], Ev::Osc(..)));
    let ev = run_with(&long_osc(4097, b"\x07"), OscLarge::Drop);
    assert_eq!(ev.len(), 1);
    assert!(matches!(&ev[0], Ev::OscLarge(p) if p.len() == 4096));
}

#[test]
fn osc_default_buffers_to_64k_then_drops() {
    // Between 4 KB and 64 KB: asked once, then dispatched whole.
    let input = long_osc(60_000, b"\x07");
    let ev = run(&input);
    assert_eq!(ev.len(), 2);
    match (&ev[0], &ev[1]) {
        (Ev::OscLarge(prefix), Ev::Osc(parts, true)) => {
            // The prefix is the raw string so far, separators included.
            assert_eq!(prefix.as_slice(), &input[2..2 + 4096]);
            assert_eq!(parts.join(&b';'), input[2..input.len() - 1].to_vec());
        },
        other => panic!("{other:?}"),
    }

    // Exactly 64 KB is kept; one byte more is dropped, not dispatched.
    let ev = run(&long_osc(64 * 1024, b"\x07"));
    assert!(matches!(ev.last(), Some(Ev::Osc(..))), "{}", ev.len());
    let ev = run(&long_osc(64 * 1024 + 1, b"\x07"));
    assert_eq!(ev.len(), 1);
    assert!(matches!(&ev[0], Ev::OscLarge(p) if p.len() == 4096));
}

#[test]
fn osc_buffer_cap_is_chosen_by_the_performer() {
    let input = long_osc(6000, b"\x07");
    let ev = run_with(&input, OscLarge::Buffer(6000));
    assert!(matches!(ev.last(), Some(Ev::Osc(..))));
    let ev = run_with(&input, OscLarge::Buffer(5999));
    assert_eq!(ev.len(), 1, "dropped past the cap");
    // A cap below what has arrived drops at once.
    let ev = run_with(&input, OscLarge::Buffer(10));
    assert_eq!(ev.len(), 1);
}

#[test]
fn osc_stream_hands_over_the_rest() {
    let input = long_osc(100_000, b"\x1b\\");
    let ev = run_with(&input, OscLarge::Stream);
    // asked once, streamed, ended; ST then arrives as an ESC dispatch.
    assert_eq!(ev.len(), 4);
    let body = &input[2..input.len() - 2];
    assert_eq!(ev[0], Ev::OscLarge(body[..4096].to_vec()));
    assert_eq!(ev[1], Ev::OscData(body[4096..].to_vec()));
    assert_eq!(ev[2], Ev::OscEnd(true));
    assert_eq!(ev[3], Ev::Esc(b'\\'));

    // BEL ends it as complete; CAN as not complete.
    let ev = run_with(&long_osc(5000, b"\x07"), OscLarge::Stream);
    assert_eq!(ev[2], Ev::OscEnd(true));
    let ev = run_with(&long_osc(5000, b"\x18"), OscLarge::Stream);
    assert_eq!(ev[2], Ev::OscEnd(false));
    assert_eq!(ev[3], Ev::Execute(0x18));
}

#[test]
fn osc_drop_discards_and_recovers() {
    let mut input = long_osc(10_000, b"\x07");
    input.extend_from_slice(b"\x1b]0;title\x07");
    let ev = run_with(&input, OscLarge::Drop);
    assert_eq!(ev.len(), 2);
    assert!(matches!(&ev[0], Ev::OscLarge(_)));
    assert_eq!(ev[1], osc(&["0", "title"], true));
    // The same after a string dropped for passing its cap.
    let ev = run_with(&input, OscLarge::Buffer(5000));
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[1], osc(&["0", "title"], true));
}

/// A mix of everything, for the equivalence test.
fn corpus() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"text \x1b[31;1mred\x1b[m \xc3\xa9\xe4\xb8\x96\r\n");
    v.extend_from_slice(b"\x1b]8;id=1;http://x/y;z\x07link\x1b]8;;\x1b\\");
    v.extend_from_slice(&long_osc(9000, b"\x07"));
    v.extend_from_slice(&long_osc(9000, b"\x1b\\"));
    v.extend_from_slice(b"\x1b_Gi=1,m=1;QUJD\x1b\\\x1b_Gm=0;REVG\x18after");
    v.extend_from_slice(b"\x1b_G\x1b[Hxx\x1b_G\x1b\x1b\\");
    v.extend_from_slice(b"\x1bPq#0;2;0;0;0#0~~~@@@\x1b\\\x1bP+q544e\x1b\\");
    v.extend_from_slice(b"\x1bPqab\x1acd\x1bPqef\xe4\x7fgh\x1b\\");
    v.extend_from_slice(b"\x1bXsos\x1b\\\x1b^pm\x1b\\");
    v.extend_from_slice(b"\x1b]2;a\x01b\x08c\x07\x1b]52;c;SGVsbG8=\x07");
    v.extend_from_slice(&long_osc(70_000, b"\x07"));
    v.extend_from_slice(b"end\x1b");
    v
}

#[test]
fn bulk_and_byte_at_a_time_give_the_same_callbacks() {
    let input = corpus();
    for policy in
        [OscLarge::Buffer(64 * 1024), OscLarge::Buffer(8000), OscLarge::Stream, OscLarge::Drop]
    {
        let whole = run_chunked(&input, input.len(), true, policy);
        // Make sure the corpus really exercises each callback.
        assert!(whole.iter().any(|e| matches!(e, Ev::Apc(_))));
        assert!(whole.iter().any(|e| matches!(e, Ev::Dcs(_))));
        for chunk in [1, 2, 3, 7, 64, 4095, 4096, 4097] {
            assert_eq!(
                run_chunked(&input, chunk, true, policy),
                whole,
                "chunk {chunk}, policy {policy:?}"
            );
        }
        // And DCS data through `put` alone.
        assert_eq!(run_chunked(&input, 1, false, policy), whole, "put only, {policy:?}");
    }
}
