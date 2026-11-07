export default function Install() {
  return (
<section className={"band install"} id={"install"} aria-labelledby={"h-install"}>
  <span className={"corner br"} aria-hidden={"true"}>{"┘"}</span>
  <div className={"wrap section-head"}>
    <div>
      <h2 id={"h-install"}>{"Install"}</h2>
      <pre className={"big"} style={{"marginTop": "2rem"}}>{"brew install --cask 0xdeafcafe/tap/terminal-decline"}</pre>
    </div>
    <div className={"copy"}>
      <p>{"Apple Silicon, macOS 14 Sonoma or later. That installs Terminal Decline.app and the command as both "}<code>{"terminal-decline"}</code>{" and "}<code>{"decline"}</code>{". MIT licence."}</p>
      <p>{"From source: clone the repository and run "}<code>{"scripts/dogfood.sh --open"}</code>{", which builds a release, makes and signs the app bundle and installs it in ~/Applications. "}<code>{"cargo install"}</code>{" gives the bare command without the bundle, so no Dock pins from the app. The "}<a href={"https://github.com/0xdeafcafe/terminal-decline#install"}>{"README"}</a>{" has both routes in full."}</p>
      <p className={"muted"}>{"The in-app updater only offers signed and notarised releases, and there are none yet."}</p>
      <ul className={"links-list"}>
        <li><a href={"https://github.com/0xdeafcafe/terminal-decline"}>{"github.com/0xdeafcafe/terminal-decline"}</a></li>
        <li><a href={"https://github.com/0xdeafcafe/terminal-decline/blob/main/DESIGN.md"}>{"DESIGN.md, how it works and the roadmap"}</a></li>
        <li><a href={"https://github.com/0xdeafcafe/terminal-decline/blob/main/CHANGELOG.md"}>{"Changelog"}</a></li>
      </ul>
    </div>
  </div>
</section>
  );
}
