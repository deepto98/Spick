import { useState } from "react";
import {
  ArrowUpRight,
  ChevronRight,
  Clock3,
  Copy,
  Gauge,
  Languages,
  Mic2,
  Sparkles,
  Timer,
  TrendingUp,
} from "lucide-react";
import { languages, recentDictations, weeklyWords } from "../data/mockData";
import type { HudState } from "../types";
import { DictationHud } from "../components/DictationHud";
import { PageHeader } from "../components/Ui";

interface TodayViewProps {
  onOpenEngines: () => void;
  hudState: HudState;
  onHudStateChange: (state: HudState) => void;
}

export function TodayView({
  onOpenEngines,
  hudState,
  onHudStateChange,
}: TodayViewProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const maxWords = Math.max(...weeklyWords.map((item) => item.words));

  const copyText = async (id: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId(null), 1200);
    } catch {
      setCopiedId(null);
    }
  };

  return (
    <div className="view view--today">
      <PageHeader
        eyebrow="INTERACTIVE PRODUCT PREVIEW"
        title="Your dictation workspace"
        description="Sample activity demonstrates the dashboard; live statistics begin when the audio pipeline is connected."
        actions={
          <button
            type="button"
            className="button button--secondary"
            onClick={onOpenEngines}
          >
            <Sparkles size={16} />
            Set up an engine
            <ChevronRight size={15} />
          </button>
        }
      />

      <section className="stat-grid" aria-label="Dictation statistics">
        <article className="stat-card stat-card--primary">
          <span className="stat-card__icon">
            <Mic2 size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>3,128</strong>
            <span>sample words</span>
          </div>
          <span className="trend trend--light">
            <TrendingUp size={13} /> 24%
          </span>
          <div className="stat-card__sparkline" aria-hidden="true">
            {[22, 35, 28, 49, 43, 62, 58, 74, 69, 87, 76, 96].map(
              (height, index) => (
                <i key={index} style={{ height: `${height}%` }} />
              ),
            )}
          </div>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Gauge size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>142</strong>
            <span>sample WPM</span>
          </div>
          <span className="trend">
            <ArrowUpRight size={13} /> 8%
          </span>
          <small>3.2× faster than typing</small>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Timer size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>28m</strong>
            <span>sample time saved</span>
          </div>
          <span className="trend">
            <ArrowUpRight size={13} /> 12%
          </span>
          <small>3h 42m saved this month</small>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Languages size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>3</strong>
            <span>sample languages</span>
          </div>
          <span
            className="language-stack"
            aria-label="English, Hindi, and Bengali"
          >
            {languages.map((language) => (
              <i key={language.code}>{language.code}</i>
            ))}
          </span>
          <small>Auto-detect is on</small>
        </article>
      </section>

      <div className="dashboard-grid">
        <section className="panel activity-panel">
          <header className="panel__header">
            <div>
              <h2>Speaking activity</h2>
              <p>Example weekly trend</p>
            </div>
            <button type="button" className="filter-button">
              This week <ChevronRight size={14} />
            </button>
          </header>
          <div className="chart-summary">
            <strong>21,680</strong>
            <span>
              <TrendingUp size={13} /> 18% from last week
            </span>
          </div>
          <div
            className="bar-chart"
            aria-label="Weekly words dictated bar chart"
          >
            <div className="bar-chart__grid" aria-hidden="true">
              <i />
              <i />
              <i />
            </div>
            {weeklyWords.map((item, index) => (
              <div className="bar-chart__item" key={item.day}>
                <span className="bar-chart__value">
                  {item.words.toLocaleString()}
                </span>
                <div className="bar-chart__track">
                  <i
                    className={
                      index === weeklyWords.length - 1
                        ? "bar-chart__bar--active"
                        : ""
                    }
                    style={{
                      height: `${Math.max(18, (item.words / maxWords) * 100)}%`,
                    }}
                  />
                </div>
                <span>{item.day}</span>
              </div>
            ))}
          </div>
        </section>

        <section className="panel language-panel">
          <header className="panel__header">
            <div>
              <h2>Languages</h2>
              <p>Example language mix</p>
            </div>
            <span className="auto-badge">
              <i /> Auto
            </span>
          </header>
          <div className="language-donut-wrap">
            <div
              className="language-donut"
              aria-label="68 percent English, 22 percent Hindi, 10 percent Bengali"
            >
              <div>
                <strong>3</strong>
                <span>languages</span>
              </div>
            </div>
          </div>
          <div className="language-legend">
            {languages.map((language) => (
              <div key={language.code}>
                <i style={{ backgroundColor: language.color }} />
                <span>{language.name}</span>
                <strong>{language.percentage}%</strong>
              </div>
            ))}
          </div>
        </section>
      </div>

      <div className="dashboard-grid dashboard-grid--bottom">
        <section className="panel recent-panel">
          <header className="panel__header">
            <div>
              <h2>Sample dictations</h2>
              <p>Representative polished output</p>
            </div>
            <span className="prototype-badge">SAMPLE DATA</span>
          </header>
          <div className="dictation-list">
            {recentDictations.map((dictation) => (
              <article className="dictation-row" key={dictation.id}>
                <span
                  className="app-tile"
                  style={{ backgroundColor: dictation.color }}
                >
                  {dictation.application[0]}
                </span>
                <div className="dictation-row__body">
                  <div className="dictation-row__meta">
                    <strong>{dictation.application}</strong>
                    <span>{dictation.timestamp}</span>
                  </div>
                  <p>{dictation.text}</p>
                  <div className="dictation-row__details">
                    <span>{dictation.language}</span>
                    <span>{dictation.words} words</span>
                  </div>
                </div>
                <button
                  type="button"
                  className="icon-button icon-button--subtle"
                  onClick={() => void copyText(dictation.id, dictation.text)}
                  aria-label={`Copy dictation from ${dictation.application}`}
                >
                  {copiedId === dictation.id ? (
                    <span className="copied-label">Copied</span>
                  ) : (
                    <Copy size={15} />
                  )}
                </button>
              </article>
            ))}
          </div>
        </section>

        <section className="panel try-panel">
          <div className="try-panel__glow" />
          <span className="try-panel__eyebrow">
            <i /> LIVE PREVIEW
          </span>
          <h2>Speak. Watch it become clear.</h2>
          <p>
            Try the animation. This preview does not capture microphone audio.
          </p>
          <div className="try-panel__hud">
            <DictationHud state={hudState} onStateChange={onHudStateChange} />
          </div>
          <div className="try-panel__footer">
            <span>
              <Clock3 size={14} /> Interaction
            </span>
            <strong>Demo only</strong>
          </div>
        </section>
      </div>
    </div>
  );
}
