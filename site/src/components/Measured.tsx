export default function Measured() {
  return (
    <section className="band field" id="measured" aria-labelledby="h-bench">
      <div className="wrap">
        <div className="section-head">
          <h2 id="h-bench">Benchmarks</h2>
          <p>2 November 2026. M1 Max, macOS 27.0. Medians of five runs. Lower is better.</p>
        </div>
    <div className={"chart after-head"}>
      <div className={"group"}>
        <h3>{"Four tabs "}<span>{"MB"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "45.2%"}} aria-hidden={"true"}></span><span className={"v"}>{"47 MB"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "83.7%"}} aria-hidden={"true"}></span><span className={"v"}>{"87 MB"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"104 MB"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"After 10k lines "}<span>{"MB"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "14.6%"}} aria-hidden={"true"}></span><span className={"v"}>{"19 MB"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "43.1%"}} aria-hidden={"true"}></span><span className={"v"}>{"56 MB"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"130 MB"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"CPU for "}<code>{"cat"}</code>{" "}<span>{"seconds, including WindowServer"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "32.4%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.66 s"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"2.04 s"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "79.9%"}} aria-hidden={"true"}></span><span className={"v"}>{"1.63 s"}</span></div>
      </div>
      <div className={"group"}>
        <h3>{"Idle CPU "}<span>{"% of one core"}</span></h3>
        <div className={"row best"}><b>{"Decline"}</b><span className={"bar"} style={{"width": "16.7%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.02%"}</span></div>
        <div className={"row"}><span>{"Ghostty"}</span><span className={"bar"} style={{"width": "25.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.03%"}</span></div>
        <div className={"row"}><span>{"kitty"}</span><span className={"bar"} style={{"width": "100.0%"}} aria-hidden={"true"}></span><span className={"v"}>{"0.12%"}</span></div>
      </div>
    </div>
        <details className="benchmark-method after-head">
          <summary>Method and earlier results</summary>
          <div>
            <p>Menlo 13, 100×30 cells, 10k lines of scrollback. One window. Memory is physical footprint across all processes. Runs were interleaved at system load 3–6. Ghostty 1.4.0 and kitty 0.49.2. <a href="https://github.com/0xdeafcafe/terminal-decline#benchmarks">Builds, script and full results</a>.</p>
            <p>On 30 October, Ghostty finished the <code>cat</code> in 0.53 s, against Decline’s 0.61 s. Neither run compared input latency. <a href="https://github.com/0xdeafcafe/terminal-decline/blob/main/docs/comparison-2026-10-30.md">30 October results</a>.</p>
          </div>
        </details>
      </div>
    </section>
  );
}
