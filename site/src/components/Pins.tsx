export default function Pins() {
  return (
<section className={"band mintband"} aria-labelledby={"h-pin"}>
  <div className={"wrap"}>
    <div className={"pins"}>
      <div>
        <h2 id={"h-pin"}>{"One command, and rush is an app in the Dock."}</h2>
        <div className={"cmd after-head"} style={{"marginTop": "2rem"}}>
          <pre>{"decline pin rush --dock"}</pre>
          <p className={"small muted"}>{"A command example, not output. It needs "}<a href={"https://github.com/0xdeafcafe/rush"}>{"rush"}</a>{", the terminal app for coding agents, installed separately with "}<code>{"brew install 0xdeafcafe/tap/rush"}</code>{"."}</p>
        </div>
        <div className={"copy"} style={{"display": "grid", "gap": "1rem", "marginTop": "2rem"}}>
          <p>{"That writes "}<code>{"~/Applications/rush.app"}</code>{" and puts it in the Dock. Click it and a Decline window opens running rush, nothing else. A pin keeps its own settings, a config layer of its own ("}<code>{"decline pin --edit rush"}</code>{"), and "}<code>{"--list"}</code>{" and "}<code>{"--remove"}</code>{" do what they say. Pins load the installed app, so they keep working after Decline updates."}</p>
          <p>{"The icon is rush's own if it ships one: an app bundle's, or "}<code>{"rush.png"}</code>{" or "}<code>{"rush.icns"}</code>{" beside the program. Otherwise Decline draws one, a large initial over the Slope on one of ten fixed hues, picked by a hash of the name. "}<code>{"--icon FILE"}</code>{" picks something else."}</p>
        </div>
      </div>
      <div>
        <div className={"pin-big"}>
          <img className={"r"} src={"/images/rush.webp"} width={512} height={512} width={"512"} height={"512"} alt={"A generated pin for Rush: a white to mint capital R above the Slope, on a teal rounded square"} loading="lazy" decoding="async" />
          <div className={"pin-sizes small"}>
            <div><img src={"/images/rush-32.webp"} width={32} height={32} width={"32"} height={"32"} alt={"The Rush pin drawn at 32 pixels"} loading="lazy" decoding="async" /><span>{"32 px"}</span></div>
            <div><img className={"s16"} src={"/images/rush-16.webp"} width={16} height={16} width={"16"} height={"16"} alt={"The Rush pin drawn at 16 pixels"} loading="lazy" decoding="async" /><span>{"16 px"}</span></div>
          </div>
        </div>
        <div className={"pin-row"} style={{"marginTop": "2.5rem"}}>
          <figure><img src={"/images/htop.webp"} width={512} height={512} width={"512"} height={"512"} alt={"A generated pin for htop: a capital H above the Slope on indigo"} loading="lazy" decoding="async" /><figcaption className={"small"}>{"htop"}</figcaption></figure>
          <figure><img src={"/images/claude.webp"} width={512} height={512} width={"512"} height={"512"} alt={"A generated pin for Claude: a capital C above the Slope on violet"} loading="lazy" decoding="async" /><figcaption className={"small"}>{"Claude"}</figcaption></figure>
          <figure><img src={"/images/logs.webp"} width={512} height={512} width={"512"} height={"512"} alt={"A generated pin for Logs: a capital L above the Slope on amber"} loading="lazy" decoding="async" /><figcaption className={"small"}>{"Logs"}</figcaption></figure>
        </div>
        <p className={"small muted"} style={{"marginTop": "1.5rem"}}>{"Generated pins, artwork rather than a photo of a Dock."}</p>
      </div>
    </div>
  </div>
</section>
  );
}
