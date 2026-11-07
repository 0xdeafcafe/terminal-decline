export default function Pins() {
  return (
    <section className="band mintband" aria-labelledby="h-pin">
      <div className="wrap pins">
        <div>
          <h2 id="h-pin">Pin a command to the Dock.</h2>
          <p className="after-head">Open rush in its own window, with its own settings.</p>
          <pre className="pin-command">decline pin rush --dock</pre>
          <p className="small muted">Requires <a href="https://github.com/0xdeafcafe/rush">rush</a>. <a href="https://github.com/0xdeafcafe/terminal-decline#pin-a-program-to-the-dock">Pin options</a>.</p>
        </div>
        <div>
          <div className="pin-big">
            <img className="r" src="/images/rush.webp" width={512} height={512} alt="Generated Rush pin: an R above the Slope" loading="lazy" decoding="async" />
            <div className="pin-sizes small">
              <div><img src="/images/rush-32.webp" width={32} height={32} alt="Rush pin at 32 pixels" loading="lazy" decoding="async" /><span>32 px</span></div>
              <div><img className="s16" src="/images/rush-16.webp" width={16} height={16} alt="Rush pin at 16 pixels" loading="lazy" decoding="async" /><span>16 px</span></div>
            </div>
          </div>
          <div className="pin-row pin-samples">
            <figure><img src="/images/htop.webp" width={512} height={512} alt="Generated htop pin" loading="lazy" decoding="async" /><figcaption className="small">htop</figcaption></figure>
            <figure><img src="/images/claude.webp" width={512} height={512} alt="Generated Claude pin" loading="lazy" decoding="async" /><figcaption className="small">Claude</figcaption></figure>
            <figure><img src="/images/logs.webp" width={512} height={512} alt="Generated Logs pin" loading="lazy" decoding="async" /><figcaption className="small">Logs</figcaption></figure>
          </div>
          <p className="small muted pin-caption">Generated pin icons.</p>
        </div>
      </div>
    </section>
  );
}
