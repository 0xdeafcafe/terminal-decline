export default function Completion() {
  return (
<section className={"band"} aria-labelledby={"h-tab"}>
  <div className={"wrap"}>
    <div className={"section-head"}>
      <h2 id={"h-tab"}>{"Every match comes with its description."}</h2>
      <div className={"copy"}>
        <p>{"Decline asks zsh for its completions, then adds a description to each flag and subcommand from the man page or the program's "}<code>{"--help"}</code>{". The terminal draws the list itself, in your font and theme."}</p>
        <p>{"Grey text after the cursor shows the rest of the first match, and Right takes it. Ctrl+Space opens the list at once. Having it open as you type is a setting ("}<code>{"completion_as_you_type"}</code>{"), off by default."}</p>
        <p className={"muted"}>{"The matches come from zsh. This page makes no promise about other shells."}</p>
      </div>
    </div>
    <figure className={"shot"}>
      <div className={"whole"}><img src={"/images/screenshot-autocomplete.webp"} width={2058} height={1574} width={"2058"} height={"1574"} alt={"A Decline window with a dark theme, in the home folder. ll listed the folders, then git fetch - was typed and Tab pressed. A list shows --all, --no-all, -a, --append, --atomic, --depth, --deepen and --shallow-since, each with the first line of its description from the git-fetch man page. --depth is highlighted."} loading="lazy" decoding="async" /></div>
      <div className={"crop"} aria-hidden={"true"}><img src={"/images/screenshot-autocomplete.webp"} width={2058} height={1574} alt={""} loading="lazy" decoding="async" /></div>
      <figcaption className={"small muted"}>{"A real window, unaltered. Cropped to the prompt and list on narrow screens."}</figcaption>
    </figure>
  </div>
</section>
  );
}
