import Image from "next/image";

const previewDownloadUrl =
  "https://github.com/deepto98/Spick/releases/download/v0.1.0-preview.1/Spick_0.1.0_local_aarch64.dmg";
const downloadUrl = process.env.SPICK_DMG_URL ?? previewDownloadUrl;

const waveBars = [18, 34, 24, 48, 28, 56, 38, 22, 44, 31, 51, 25, 37];

export default function Home() {
  return (
    <main>
      <nav className="nav shell" aria-label="Main navigation">
        <a className="wordmark" href="#top" aria-label="Spick home">
          <Image
            src="/spick-mark.png"
            alt=""
            width={38}
            height={38}
            unoptimized
          />
          <span>Spick</span>
        </a>
        <div className="navLinks">
          <a href="#how">How it works</a>
          <a href="#privacy">Privacy</a>
          <a className="navDownload" href="#download">
            Get the Mac app
          </a>
        </div>
      </nav>

      <section className="hero shell" id="top">
        <div className="heroCopy">
          <p className="eyebrow">Voice typing for macOS</p>
          <h1>Keep your hands on the work.</h1>
          <p className="lede">
            Press Option. Say the sentence. Spick puts it back where your cursor
            was—without the ums, false starts, or detour through another app.
          </p>
          <div className="heroActions">
            <a className="button buttonPrimary" href="#download">
              Get Spick for Mac <span aria-hidden="true">↓</span>
            </a>
            <span className="buildNote">Apple silicon · private preview</span>
          </div>
          <p className="scribble">speak it, then carry on</p>
        </div>

        <div className="productStage" aria-label="Preview of the Spick app">
          <div className="appWindow">
            <aside className="mockSidebar">
              <div className="mockBrand">
                <Image
                  src="/spick-mark.png"
                  alt=""
                  width={29}
                  height={29}
                  unoptimized
                />
                <span>Spick</span>
              </div>
              <div className="mockNav active">Stats</div>
              <div className="mockNav">Notes</div>
              <div className="mockNav">Engines</div>
              <div className="mockNav">Vocabulary</div>
            </aside>
            <div className="mockMain">
              <div className="mockTopline">
                <span>Stats</span>
                <kbd>⌥</kbd>
              </div>
              <p className="mockHello">A quiet record of words spoken.</p>
              <div className="statGrid">
                <article>
                  <small>Words spoken</small>
                  <strong>2,842</strong>
                  <i className="rise">↑ 18%</i>
                </article>
                <article>
                  <small>Speaking pace</small>
                  <strong>142</strong>
                  <i>words / min</i>
                </article>
                <article className="wideStat">
                  <small>This week</small>
                  <div className="miniBars" aria-hidden="true">
                    {[35, 58, 42, 74, 49, 86, 68].map((height, index) => (
                      <i key={index} style={{ height: `${height}%` }} />
                    ))}
                  </div>
                </article>
              </div>
            </div>
          </div>
          <div className="edgeWidget" aria-label="Compact listening widget">
            {waveBars.map((height, index) => (
              <i
                key={index}
                style={{ width: `${Math.max(3, height / 7)}px` }}
              />
            ))}
          </div>
          <span className="stageNote">stays out of your way</span>
        </div>
      </section>

      <section className="workStrip" aria-label="Works across your Mac">
        <div className="shell workStripInner">
          <span>Works where your cursor works</span>
          <div className="appList" aria-label="Example applications">
            <b>Notes</b>
            <i>·</i>
            <b>Chrome</b>
            <i>·</i>
            <b>VS Code</b>
            <i>·</i>
            <b>Slack</b>
            <i>·</i>
            <b>Mail</b>
          </div>
        </div>
      </section>

      <section className="how shell" id="how">
        <div className="sectionIntro">
          <p className="eyebrow">Nothing new to learn</p>
          <h2>Option down. Words out.</h2>
          <p>
            Hold Option while you talk, or tap once to start and once to stop.
            The little bar at the edge of your screen lets you know Spick is
            listening.
          </p>
        </div>
        <div className="steps">
          <article>
            <span>01</span>
            <kbd>⌥</kbd>
            <h3>Stay in the field</h3>
            <p>No recorder window. No copy-and-paste routine.</p>
          </article>
          <article>
            <span>02</span>
            <div className="talkWave" aria-hidden="true">
              ⌁⌁⌁
            </div>
            <h3>Talk like yourself</h3>
            <p>Choose verbatim, or tidy the pauses and rough edges.</p>
          </article>
          <article>
            <span>03</span>
            <div className="caretMark" aria-hidden="true">
              ab│
            </div>
            <h3>Find it at the cursor</h3>
            <p>Your sentence returns to the app where you started.</p>
          </article>
        </div>
      </section>

      <section className="privacy" id="privacy">
        <div className="shell privacyGrid">
          <div>
            <p className="eyebrow light">Private by default</p>
            <h2>Your Mac can do the listening.</h2>
          </div>
          <div className="privacyCopy">
            <p>
              Spick ships with a small multilingual Whisper model, so the first
              word can stay on your machine. Raw audio is temporary. Transcript
              history is off until you choose otherwise.
            </p>
            <ul>
              <li>Local transcription works offline</li>
              <li>Cloud providers are an explicit choice</li>
              <li>Secure and password fields are left alone</li>
            </ul>
          </div>
        </div>
      </section>

      <section className="details shell">
        <article>
          <span className="detailMark">Aa</span>
          <h3>Many languages, one shortcut</h3>
          <p>Auto-detect or pin a language from the edge widget.</p>
        </article>
        <article>
          <span className="detailMark">#</span>
          <h3>Notes made for speaking</h3>
          <p>Draft in Markdown, preview it, and save as .md or .txt.</p>
        </article>
        <article>
          <span className="detailMark">↔</span>
          <h3>Your choice of engine</h3>
          <p>Use local Whisper or bring an OpenAI, Gemini, or xAI key.</p>
        </article>
      </section>

      <section className="download shell" id="download">
        <Image
          src="/spick-mark.png"
          alt=""
          width={98}
          height={98}
          unoptimized
        />
        <div>
          <p className="eyebrow">Spick for Mac</p>
          <h2>Give the keyboard a breather.</h2>
          <p>
            The Apple silicon preview is ready. A signed, notarized public build
            will replace it before wider release.
          </p>
        </div>
        {downloadUrl ? (
          <a className="button buttonDark" href={downloadUrl}>
            Download for Mac
          </a>
        ) : (
          <span className="button buttonWaiting" aria-disabled="true">
            Signed build coming
          </span>
        )}
      </section>

      <footer className="shell footer">
        <a className="wordmark" href="#top">
          <Image
            src="/spick-mark.png"
            alt=""
            width={38}
            height={38}
            unoptimized
          />
          <span>Spick</span>
        </a>
        <p>Made for sentences that arrive faster than fingers.</p>
        <span>macOS preview · 2026</span>
      </footer>
    </main>
  );
}
