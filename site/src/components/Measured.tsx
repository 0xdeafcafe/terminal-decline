export default function Measured() {
  return (
<section className={"band field"} id={"measured"} aria-labelledby={"h-bench"}>
  <div className={"wrap"}>
    <div className={"section-head"}>
      <div>
        <h2 id={"h-bench"}>{"What it was measured on, and the rest of the numbers."}</h2>
        <pre className={"ghost ghost-b"} aria-hidden={"true"}>{"  socket                        print the socket's path\n  hello                         who serves (app or headless)\n  spawn [OPTIONS] [-- PROGRAM [ARGS...]]\n        --name NAME             the session's id (default 1, 2, ...)\n        --size COLSxROWS        default 80x24\n        --cwd DIR               default the current directory\n        --env KEY=VALUE         more environment (yours is passed on)\n        --term TERM             default xterm-256color"}</pre>
      </div>
      <div className={"copy"}>
        <p>{"2 November 2026, Apple M1 Max, macOS 27.0. Menlo 13, a 100×30 window with 10k lines of scrollback, one window. Memory is macOS physical footprint summed over all of the terminal's processes. Medians of 5 runs, taken in turns, on a Mac with other work running (load 3 to 6, which is to say an ordinary afternoon)."}</p>
        <p className={"muted"}>{"Ghostty 1.4.0, kitty 0.49.2. The builds measured, the script and the full table are in the "}<a href={"https://github.com/0xdeafcafe/terminal-decline#benchmarks"}>{"README"}</a>{"."}</p>
      </div>
    </div>
    <div className={"chart after-head"}>
      <div className={"group"}>
        <h3>{"Memory with 4 tabs "}<span>{"MB, lower is better"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "45.2%"}} aria-hidden={"true"}></span><span className={"v"}>{"47 MB"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "83.7%"}} aria-hidden={"true"}></span><span className={"v"}>{"87 MB"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"104 MB"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"Memory after 10k lines "}<span>{"MB, lower is better"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "14.6%"}} aria-hidden={"true"}></span><span className={"v"}>{"19 MB"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "43.1%"}} aria-hidden={"true"}></span><span className={"v"}>{"56 MB"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"130 MB"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"CPU for that "}<code>{"cat"}</code>{", WindowServer included "}<span>{"seconds, lower is better"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "32.4%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.66 s"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"2.04 s"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "79.9%"}} aria-hidden={"true"}></span><span className={"v"}>{"1.63 s"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"Idle CPU "}<span>{"percent of a core, lower is better"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "16.7%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.02%"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "25.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.03%"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.12%"}</span></div>
      </div>
    </div>
    <div className={"section-head after-head"}>
      <div></div>
      <div className={"copy"}>
        <p>{"Idle, Decline woke 0.5 times a second, as Ghostty did, and kitty 24. Decline's 0.02% CPU is not zero."}</p>
        <p className={"muted"}>{"An earlier run on 30 October, which also had Warp and Terminal.app in it, is kept apart and not merged into these numbers. In that one Decline idled at 31.5 MB and took 0.61 s for the "}<code>{"cat"}</code>{", and Ghostty's "}<code>{"cat"}</code>{" was quicker, at 0.53 s. Warp (running two windows) took 3.6 s and Terminal.app 4.0 s. None of these runs compared input latency between terminals. "}<a href={"https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/comparison-2026-10-30.md"}>{"The 30 October write-up"}</a>{" has that table."}</p>
      </div>
    </div>
  </div>
</section>
  );
}
