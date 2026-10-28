#!/usr/bin/env python3
"""Works out a crash signature from a cargo-fuzz log, to deduplicate issues.

The signature is the panic message (or the libFuzzer/sanitizer error) with
numbers replaced by N, plus the top stack frame in our own code. Line
numbers and hashes are left out, so the same bug found on two days after
unrelated edits gets the same signature.

usage: fuzz-signature.py LOG [--report OUT.md --input RAW --minimized MIN --run-url URL]
       fuzz-signature.py --self-test

Prints JSON {signature, title, message, frame}; with $GITHUB_OUTPUT set, also
writes `signature` and `title` there. --report writes the issue body.
"""
import argparse
import hashlib
import json
import os
import re
import sys

PANIC = re.compile(r"panicked at (?P<loc>[^\n]*?):\d+:\d+:\n(?P<msg>[^\n]*)")
PANIC_OLD = re.compile(r"panicked at '(?P<msg>[^\n]*?)', (?P<loc>[^\n]*?):\d+:\d+")
RUST_FRAME = re.compile(r"^\s*\d+:\s+(?:0x[0-9a-f]+ - )?(?P<fn>[^\s].*?)\s*$", re.M)
FRAME = re.compile(r"^\s*#\d+\s+0x[0-9a-f]+\s+in\s+(?P<fn>.+?)(?:\s+(?:/|\(|[A-Za-z]:\\|\S+\.rs:).*)?$", re.M)
ERROR = re.compile(r"==\d+==\s*ERROR: (?P<msg>[^\n]*)|^(?P<lf>ERROR: libFuzzer: [^\n]*)", re.M)
SKIP = re.compile(
    r"^(_+sanitizer|__asan|__rust|__rustc::|_?rust_begin_unwind|rust_panic|fuzzer::|std::|core::|alloc::|libfuzzer_sys::|"
    r"panic_unwind|panic_abort|backtrace::|_?abort|_?raise|_?pthread_kill|_sigtramp|<unknown>|"
    r"LLVMFuzzerTestOneInput|main$|start$|__libc|libc\.so|\?\?)"
)


def normalise(msg):
    msg = re.sub(r"0x[0-9a-fA-F]+", "0xN", msg)
    msg = re.sub(r"\d+", "N", msg)
    return msg.strip()


def top_frame(log):
    # Rust's own backtrace (RUST_BACKTRACE=1) first, then the sanitizer's.
    panic_at = log.find("panicked at")
    rust = RUST_FRAME.finditer(log[panic_at:]) if panic_at >= 0 else iter(())
    for m in list(rust) + list(FRAME.finditer(log)):
        fn = re.sub(r"::h[0-9a-f]{16}$", "", m.group("fn").strip())
        if not SKIP.match(fn):
            return fn
    return None


def signature(log):
    m = PANIC.search(log) or PANIC_OLD.search(log)
    if m:
        message = m.group("msg").strip()
        loc = os.path.basename(m.group("loc"))
    else:
        e = ERROR.search(log)
        message = (e.group("msg") or e.group("lf")).strip() if e else "unknown crash"
        message = re.sub(r"^ERROR: ", "", message)
        loc = None
    frame = top_frame(log) or loc or "unknown"
    key = f"{normalise(message)}\n{frame}"
    sig = hashlib.sha1(key.encode()).hexdigest()[:12]
    short = message if len(message) <= 80 else message[:77] + "..."
    return {"signature": sig, "title": f"fuzz(parse): {short} in {frame}", "message": message, "frame": frame}


def report(sig, log, args):
    excerpt_start = max(log.find("panicked at"), 0) if "panicked at" in log else max(log.rfind("==ERROR"), 0)
    excerpt = log[excerpt_start:excerpt_start + 6000]
    parts = [
        "The nightly fuzz run found a crash in the `parse` target.",
        "",
        f"- Message: `{sig['message']}`",
        f"- Top frame: `{sig['frame']}`",
    ]
    if args.run_url:
        parts.append(f"- Run and artifacts (raw input, minimised input, log): {args.run_url}")
    for label, path in (("Minimised input", args.minimized), ("Raw input", args.input)):
        if path and os.path.exists(path):
            with open(path, "rb") as f:
                data = f.read()
            if len(data) <= 4096:
                parts += ["", f"{label} ({len(data)} bytes, as a Rust byte string):", "", "```", 'b"' + escape(data) + '"', "```"]
                break
    parts += [
        "",
        "Reproduce: download the input, then `cargo +nightly fuzz run parse <file>`; "
        "print it as steps with `cargo +nightly fuzz fmt parse <file>`. Once fixed, commit the "
        "minimised input to `fuzz/regressions/parse/` so every PR replays it.",
        "",
        "<details><summary>Log excerpt</summary>",
        "",
        "```",
        excerpt.strip(),
        "```",
        "</details>",
        "",
        f"<!-- fuzz-signature: {sig['signature']} -->",
    ]
    return "\n".join(parts) + "\n"


def escape(data):
    out = []
    for b in data:
        if b == 0x5C:
            out.append("\\\\")
        elif b == 0x22:
            out.append('\\"')
        elif 0x20 <= b < 0x7F:
            out.append(chr(b))
        else:
            out.append(f"\\x{b:02x}")
    return "".join(out)


def self_test():
    log = (
        "INFO: Running with entropic power schedule\n"
        "thread '<unnamed>' panicked at /home/runner/work/x/crates/term-core/src/term.rs:812:21:\n"
        "index out of bounds: the len is 80 but the index is 83\n"
        "note: run with `RUST_BACKTRACE=1`\n"
        "==4242== ERROR: libFuzzer: deadly signal\n"
        "    #0 0x55d1 in __sanitizer_print_stack_trace /rustc/llvm/asan.cpp:87:3\n"
        "    #1 0x55d2 in fuzzer::PrintStackTrace() /x/FuzzerUtil.cpp:210:5\n"
        "    #6 0x55d7 in std::panicking::rust_panic_with_hook::h0123456789abcdef /rustc/std/src/panicking.rs:1\n"
        "    #8 0x55d9 in core::panicking::panic_bounds_check /rustc/core/src/panicking.rs:2\n"
        "    #9 0x55da in term_core::term::Terminal::insert_chars::h0123456789abcdef /home/runner/work/x/crates/term-core/src/term.rs:812:21\n"
        "    #10 0x55db in <term_core::term::Terminal as vte::Perform>::csi_dispatch /x/term.rs:900:1\n"
    )
    a = signature(log)
    assert a["frame"] == "term_core::term::Terminal::insert_chars", a
    assert a["message"] == "index out of bounds: the len is 80 but the index is 83", a
    b = signature(log.replace("80 but the index is 83", "24 but the index is 30").replace(":812:21", ":815:9"))
    assert a["signature"] == b["signature"], (a, b)
    c = signature(log.replace("index out of bounds", "attempt to subtract with overflow"))
    assert a["signature"] != c["signature"]
    rb = signature(
        "thread '<unnamed>' (123) panicked at crates/term-core/src/term.rs:5:9:\nboom 3\nstack backtrace:\n"
        "   0: __rustc::rust_begin_unwind\n             at /rustc/x/library/std/src/panicking.rs:697:5\n"
        "   1: core::panicking::panic_fmt\n   2: term_core::term::Terminal::resize\n             at ./src/term.rs:5:9\n"
    )
    assert rb["frame"] == "term_core::term::Terminal::resize" and rb["message"] == "boom 3", rb
    t = signature("==1== ERROR: libFuzzer: timeout after 10 seconds\n    #0 0x1 in term_core::term::Terminal::scroll_up /x.rs:1:1\n")
    assert t["message"] == "libFuzzer: timeout after 10 seconds" and t["frame"] == "term_core::term::Terminal::scroll_up", t
    assert escape(b'a"\\\x1b\xff') == 'a\\"\\\\\\x1b\\xff'
    print("fuzz-signature self-test: ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("log", nargs="?")
    ap.add_argument("--report")
    ap.add_argument("--input")
    ap.add_argument("--minimized")
    ap.add_argument("--run-url")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.log:
        ap.error("LOG is required")
    with open(args.log, errors="replace") as f:
        log = f.read()
    sig = signature(log)
    print(json.dumps(sig))
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            f.write(f"signature={sig['signature']}\n")
            f.write(f"title={sig['title'].replace(chr(10), ' ')}\n")
    if args.report:
        with open(args.report, "w") as f:
            f.write(report(sig, log, args))
    return 0


if __name__ == "__main__":
    sys.exit(main())
