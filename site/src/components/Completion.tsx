export default function Completion() {
  return (
    <section className="band" aria-labelledby="h-tab">
      <div className="wrap">
        <div className="section-head">
          <h2 id="h-tab">Tab explains the flags.</h2>
          <p>Completions from zsh, with descriptions from man pages and <code>--help</code>.</p>
        </div>
        <figure className="shot">
          <div className="whole"><img src="/images/screenshot-autocomplete.webp" width={2058} height={1574} alt="git fetch completions in Decline, with a description beside each flag" loading="lazy" decoding="async" /></div>
          <div className="crop"><img src="/images/screenshot-autocomplete.webp" width={2058} height={1574} alt="git fetch completions with descriptions; cropped to the prompt and list" loading="lazy" decoding="async" /></div>
          <figcaption className="small muted"><code>git fetch</code> completions. <a href="/images/screenshot-autocomplete.png">Full window</a>.</figcaption>
        </figure>
      </div>
    </section>
  );
}
