//! The details window.
//!
//! Rendered in the system webview so it can share the visual language of the
//! project's website rather than inventing a second one, and so the evidence
//! behind the estimate — every sample, every login gap — can be laid out
//! properly instead of squeezed into menu rows.
//!
//! The document is built in two pieces: [`document`] for the initial load and
//! [`body`] for a live refresh, so an update can swap the content without
//! re-parsing the stylesheet or losing the scroll position's context.

use gcloud_dot_core::{
    credentials::AdcKind,
    settings::Theme,
    status::{ago, long_duration, AuthState, Level, Status},
    State,
};

/// Everything the page needs, resolved before rendering so the template has no
/// logic in it beyond iteration.
pub struct PanelView {
    pub level: Level,
    pub headline: String,
    pub sub: String,
    pub rows: Vec<(String, String)>,
    pub estimate_hours: f64,
    pub estimate_source: String,
    pub evidence: Evidence,
    pub samples: Vec<f64>,
    pub logins: Vec<(String, String, String)>,
    pub version: String,
}

/// How much the session-length figure is actually worth, which decides what the
/// panel says underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Three or more observed expiries: the median of real measurements.
    Measured,
    /// One or two. Real measurements, but not yet stable.
    Settling,
    /// No expiry has been observed; the number comes from login gaps or a
    /// constant.
    Inferred,
}

pub fn view(status: &Status, state: &State) -> PanelView {
    let now = chrono::Local::now();
    let level = status.level(now);

    let headline = match &status.auth {
        AuthState::Valid => match status.remaining(now) {
            Some(left) if left.num_seconds() >= 0 => long_duration(left),
            Some(_) => "Overdue".to_string(),
            None => "Signed in".to_string(),
        },
        AuthState::Expired => "Signed out".to_string(),
        AuthState::Unknown(_) => "Unknown".to_string(),
    };

    let sub = match &status.auth {
        AuthState::Valid if status.remaining(now).is_some_and(|r| r.num_seconds() >= 0) => {
            "estimated until re-auth".to_string()
        }
        _ => status.summary(now),
    };

    let mut rows = Vec::new();
    if let Some(cfg) = &status.config {
        if let Some(a) = &cfg.account {
            rows.push(("Account".into(), a.clone()));
        }
        if let Some(p) = &cfg.project {
            rows.push(("Project".into(), p.clone()));
        }
        rows.push(("Configuration".into(), cfg.name.clone()));
    }
    if let Some(start) = status.session_start {
        rows.push((
            "Last login".into(),
            format!("{} · {}", start.format("%a %b %-d, %H:%M"), ago(start, now)),
        ));
    }
    if status.auth == AuthState::Valid {
        if let Some(expiry) = status.predicted_expiry() {
            rows.push((
                "Est. re-auth".into(),
                expiry.format("%a %b %-d, %H:%M").to_string(),
            ));
        }
    }
    if let (Some(file), Some(adc)) = (&status.adc_file, &status.adc) {
        let kind = match &file.kind {
            AdcKind::UserCredentials => "user credentials".to_string(),
            AdcKind::ServiceAccount { client_email } => client_email.clone(),
            AdcKind::Other { kind } => kind.clone(),
        };
        let word = match adc {
            AuthState::Valid => "valid",
            AuthState::Expired => "expired",
            AuthState::Unknown(_) => "unknown",
        };
        rows.push(("App default creds".into(), format!("{word} · {kind}")));
    }
    if let Some(checked) = status.checked_at {
        rows.push((
            "Last checked".into(),
            checked.format("%H:%M:%S").to_string(),
        ));
    }

    // Newest first, with the gap to the previous login, which is what the
    // fallback estimator reads when no session has been seen ending.
    let ordered: Vec<_> = state.logins.iter().rev().take(10).collect();
    let logins = ordered
        .iter()
        .enumerate()
        .map(|(i, when)| {
            let gap = ordered
                .get(i + 1)
                .map(|prev| format!("+{:.1}h", (**when - **prev).num_minutes() as f64 / 60.0))
                .unwrap_or_default();
            (
                when.format("%b %-d").to_string(),
                when.format("%H:%M").to_string(),
                gap,
            )
        })
        .collect();

    use gcloud_dot_core::estimate::EstimateSource;
    let evidence = match status.estimate.source {
        EstimateSource::Observed { .. } => Evidence::Measured,
        EstimateSource::FewObservations { .. } => Evidence::Settling,
        _ => Evidence::Inferred,
    };

    PanelView {
        level,
        headline,
        sub,
        rows,
        estimate_hours: status.estimate.hours,
        estimate_source: status.estimate.source.label(),
        evidence,
        samples: state.samples.clone(),
        logins,
        version: gcloud_dot_core::VERSION.to_string(),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn rgb_css(level: Level) -> String {
    let (r, g, b) = level.rgb();
    format!("rgb({r},{g},{b})")
}

/// Three-state theming, matching the website exactly.
///
/// Light tokens live on bare `:root`. Dark is applied twice: once under
/// `prefers-color-scheme` guarded so an explicit light choice wins, and once
/// under `[data-theme="dark"]` so an explicit dark choice wins over a light
/// system. Any token defined only inside a media query would leave the panel
/// borrowing whatever the host webview happens to paint.
const PANEL_CSS: &str = r#"
:root{
  --wash-a:#d4f2e1; --wash-b:#fdeed0; --page:#f5f8f6;
  --glass:rgba(255,255,255,.66); --glass-line:rgba(255,255,255,.9);
  --text:#11150f; --muted:#556053; --faint:#818b7f;
  --rule:rgba(0,0,0,.08); --soft:rgba(0,0,0,.05); --soft-line:rgba(0,0,0,.1);
  --shadow:0 10px 30px rgba(20,60,40,.12);
  --brand:#0e8a45; --brand-ink:#fff;
}
@media (prefers-color-scheme: dark){
  :root:not([data-theme="light"]){
    --wash-a:#0d3b28; --wash-b:#3b2d0e; --page:#080a09;
    --glass:rgba(255,255,255,.055); --glass-line:rgba(255,255,255,.1);
    --text:#f2f6f2; --muted:#a4b0a6; --faint:#7c887e;
    --rule:rgba(255,255,255,.09); --soft:rgba(255,255,255,.07); --soft-line:rgba(255,255,255,.15);
    --shadow:0 18px 44px rgba(0,0,0,.55);
    --brand:#22a75a; --brand-ink:#fff;
  }
}
:root[data-theme="dark"]{
  --wash-a:#0d3b28; --wash-b:#3b2d0e; --page:#080a09;
  --glass:rgba(255,255,255,.055); --glass-line:rgba(255,255,255,.1);
  --text:#f2f6f2; --muted:#a4b0a6; --faint:#7c887e;
  --rule:rgba(255,255,255,.09); --soft:rgba(255,255,255,.07); --soft-line:rgba(255,255,255,.15);
  --shadow:0 18px 44px rgba(0,0,0,.55);
  --brand:#22a75a; --brand-ink:#fff;
}

*{box-sizing:border-box}
html,body{margin:0;height:100%}
body{
  color:var(--text); background-color:var(--page);
  background-image:
    radial-gradient(70% 52% at 12% 0, var(--wash-a) 0, transparent 62%),
    radial-gradient(62% 48% at 95% 4%, var(--wash-b) 0, transparent 58%);
  background-attachment:fixed;
  font:14px/1.5 -apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",Roboto,sans-serif;
  -webkit-font-smoothing:antialiased; user-select:none;
  /* One column with a single scrolling region, so the actions can be pinned
     rather than pushed below the fold by evidence that grows without bound. */
  display:flex; flex-direction:column; overflow:hidden;
}
.scroll{flex:1; overflow-y:auto; overflow-x:hidden; padding:16px 16px 4px}
/* No scrollbar furniture. The window is resizable and the actions are pinned,
   so a permanent grey track next to a 400 point panel is all cost. Scrolling
   by wheel, trackpad, and keyboard is untouched. */
.scroll{scrollbar-width:none; -ms-overflow-style:none}
.scroll::-webkit-scrollbar{width:0; height:0; display:none}

.bottom{
  padding:11px 16px 13px; border-top:1px solid var(--rule);
  background:var(--glass);
  backdrop-filter:blur(20px) saturate(170%); -webkit-backdrop-filter:blur(20px) saturate(170%);
}
.card{
  background:var(--glass); border:1px solid var(--glass-line); border-radius:16px;
  backdrop-filter:blur(20px) saturate(170%); -webkit-backdrop-filter:blur(20px) saturate(170%);
  box-shadow:var(--shadow); padding:16px; margin-bottom:12px;
}
.head{display:flex; align-items:center; gap:13px}
.dot{width:15px; height:15px; border-radius:50%; background:var(--level); flex:0 0 auto;
     box-shadow:0 0 0 4px color-mix(in srgb, var(--level) 20%, transparent)}
.headline{font-size:27px; font-weight:640; letter-spacing:-.022em; line-height:1.1}
.sub{color:var(--muted); font-size:12.5px; margin-top:3px}
/* minmax(0,1fr), not 1fr: a track's automatic minimum is its content size, so
   a long account address would widen the row rather than wrap inside it. */
.grid{display:grid; grid-template-columns:auto minmax(0,1fr); gap:7px 16px;
      margin-top:15px; padding-top:14px; border-top:1px solid var(--rule); font-size:13px}
.k{color:var(--faint); white-space:nowrap}
.v{text-align:right; overflow-wrap:anywhere; font-variant-numeric:tabular-nums}
h2{font-size:11px; text-transform:uppercase; letter-spacing:.075em; color:var(--faint);
   margin:0 0 10px; font-weight:640}
.est{display:flex; align-items:baseline; gap:9px; flex-wrap:wrap}
.est b{font-size:23px; font-weight:640; letter-spacing:-.02em; font-variant-numeric:tabular-nums}
.est span{color:var(--muted); font-size:12.5px}
.note{color:var(--faint); font-size:11.5px; margin-top:8px; line-height:1.45}
.spark{display:flex; align-items:flex-end; gap:3px; height:52px; margin-top:13px}
.spark i{flex:1; background:var(--brand); opacity:.5; border-radius:2px 2px 0 0}
.spark i:last-child{opacity:1}
.sparkscale{display:flex; justify-content:space-between; color:var(--faint);
            font-size:10.5px; margin-top:5px; font-variant-numeric:tabular-nums}
ul{list-style:none; margin:0; padding:0; font-size:12.5px}
li{display:flex; gap:10px; padding:5px 0; border-bottom:1px solid var(--rule);
   font-variant-numeric:tabular-nums}
li:last-child{border-bottom:0}
.d{width:52px; color:var(--muted)}
.t{width:46px}
.g{margin-left:auto; color:var(--faint)}
.actions{display:flex; gap:9px}
button{
  flex:1; font:inherit; font-weight:560; font-size:13px; padding:9px 12px; cursor:pointer;
  border-radius:10px; border:1px solid var(--soft-line); background:var(--soft); color:var(--text);
  transition:transform .06s ease, background .12s ease;
}
button:hover{background:color-mix(in srgb, var(--brand) 12%, var(--soft))}
button:active{transform:translateY(1px)}
/* The primary action is always brand green, never the status colour. Painting
   "Sign in" red because the session expired reads as a destructive button. */
button.primary{background:var(--brand); border-color:transparent; color:var(--brand-ink)}
button.primary:hover{filter:brightness(1.08)}
footer{text-align:center; color:var(--faint); font-size:11px; margin-top:9px}
footer a{color:var(--faint)}
"#;

/// The scrolling content and the pinned action bar.
pub fn body(v: &PanelView) -> String {
    let rows: String = v
        .rows
        .iter()
        .map(|(k, val)| {
            format!(
                "<div class=\"k\">{}</div><div class=\"v\">{}</div>",
                esc(k),
                esc(val)
            )
        })
        .collect();

    // Sample bars, scaled across the observed range rather than from zero:
    // eighteen samples between 15.91 and 16.08 all look identical against a
    // zero baseline, and the spread is the interesting part.
    let bars = if v.samples.len() < 2 {
        String::new()
    } else {
        let min = v.samples.iter().cloned().fold(f64::MAX, f64::min);
        let max = v.samples.iter().cloned().fold(f64::MIN, f64::max);
        let span = (max - min).max(0.0001);
        let inner: String = v
            .samples
            .iter()
            .map(|s| {
                let h = 18.0 + ((s - min) / span) * 34.0;
                format!("<i style=\"height:{h:.1}px\" title=\"{s:.2}h\"></i>")
            })
            .collect();
        format!(
            "<div class=\"spark\">{inner}</div>\
             <div class=\"sparkscale\"><span>{min:.2}h</span><span>{max:.2}h</span></div>"
        )
    };

    let logins: String = v
        .logins
        .iter()
        .map(|(day, time, gap)| {
            format!(
                "<li><span class=\"d\">{}</span><span class=\"t\">{}</span><span class=\"g\">{}</span></li>",
                esc(day), esc(time), esc(gap)
            )
        })
        .collect();

    // Must agree with the figure above it. Saying "not yet measured" under a
    // label reading "measured, n=1, settling" is a straight contradiction, and
    // it shipped in the first build.
    let note = match v.evidence {
        Evidence::Measured => "Measured from sessions seen expiring.",
        Evidence::Settling => {
            "Measured, but from too few sessions to be steady yet. It sharpens with each expiry observed."
        }
        Evidence::Inferred => {
            "Not yet measured. Inferred from your login history, and replaced as soon as a session is seen expiring."
        }
    };

    format!(
        // r##: the template contains href="#", and `"#` would close an r#"..."# literal.
        r##"<div class="scroll">
  <div class="card">
    <div class="head">
      <div class="dot"></div>
      <div>
        <div class="headline">{headline}</div>
        <div class="sub">{sub}</div>
      </div>
    </div>
    <div class="grid">{rows}</div>
  </div>

  <div class="card">
    <h2>Session length</h2>
    <div class="est"><b>{hours:.2}h</b><span>{source}</span></div>
    {bars}
    <div class="note">{note}</div>
  </div>

  <div class="card">
    <h2>Recent logins</h2>
    <ul>{logins}</ul>
  </div>
</div>

<div class="bottom">
  <div class="actions">
    <button class="primary" onclick="send('login')">Sign in</button>
    <button onclick="send('check')">Check now</button>
  </div>
  <footer>GCloud Dot {version} · <a href="#" onclick="send('website');return false">website</a></footer>
</div>"##,
        headline = esc(&v.headline),
        sub = esc(&v.sub),
        rows = rows,
        hours = v.estimate_hours,
        source = esc(&v.estimate_source),
        bars = bars,
        note = note,
        logins = logins,
        version = esc(&v.version),
    )
}

/// The complete document, for the initial load.
pub fn document(v: &PanelView, theme: Theme) -> String {
    let attr = theme.attr();
    let theme_attr = if attr.is_empty() {
        String::new()
    } else {
        format!(" data-theme=\"{attr}\"")
    };
    format!(
        r#"<!doctype html>
<html lang="en"{theme_attr}>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>GCloud Dot</title>
<style>{css}
:root{{--level:{level}}}</style>
</head>
<body>
{body}
<script>
  function send(action) {{ window.ipc.postMessage(JSON.stringify({{action}})); }}
</script>
</body>
</html>"#,
        theme_attr = theme_attr,
        css = PANEL_CSS,
        level = rgb_css(v.level),
        body = body(v),
    )
}

/// Script that swaps the content of an open panel in place.
///
/// `send` survives because it was defined on `window` by the initial load;
/// replacing the body removes the old `<script>` element but not the global it
/// already created.
pub fn refresh_script(v: &PanelView, theme: Theme) -> String {
    let attr = theme.attr();
    format!(
        "document.documentElement.setAttribute('data-theme', {theme});\
         document.documentElement.style.setProperty('--level', {level});\
         document.body.innerHTML = {body};",
        theme = serde_json::to_string(attr).unwrap_or_else(|_| "''".into()),
        level = serde_json::to_string(&rgb_css(v.level)).unwrap_or_else(|_| "''".into()),
        body = serde_json::to_string(&body(v)).unwrap_or_else(|_| "''".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local};
    use gcloud_dot_core::{
        config::ActiveConfig,
        estimate::{Estimate, EstimateSource},
    };

    fn fixture() -> (Status, State) {
        let mut state = State::default();
        let base = Local::now() - Duration::hours(40);
        for i in 0..3 {
            state.logins.push(base + Duration::hours(i * 16));
        }
        for s in [15.91, 16.07, 16.06] {
            state.record_sample(s);
        }
        let status = Status {
            gcloud_found: true,
            auth: AuthState::Valid,
            config: Some(ActiveConfig {
                name: "default".into(),
                account: Some("nic@glazkov.com".into()),
                project: Some("my-project".into()),
            }),
            session_start: state.last_login(),
            estimate: Estimate {
                hours: 16.06,
                source: EstimateSource::Observed { count: 3 },
            },
            checked_at: Some(Local::now()),
            ..Default::default()
        };
        (status, state)
    }

    #[test]
    fn renders_the_account_and_evidence() {
        let (status, state) = fixture();
        let page = document(&view(&status, &state), Theme::System);
        assert!(page.contains("nic@glazkov.com"));
        assert!(page.contains("my-project"));
        assert!(page.contains("16.06"));
        assert!(page.contains("Measured from sessions seen expiring."));
    }

    #[test]
    fn the_note_never_contradicts_the_label() {
        // The first build showed "measured, n=1, settling" above a note reading
        // "Not yet measured", which is a straight contradiction.
        let (mut status, state) = fixture();
        status.estimate = Estimate {
            hours: 16.0,
            source: EstimateSource::FewObservations { count: 1 },
        };
        let v = view(&status, &state);
        assert_eq!(v.evidence, Evidence::Settling);
        let page = document(&v, Theme::System);
        assert!(page.contains("measured, n=1, settling"));
        assert!(
            !page.contains("Not yet measured"),
            "a measured figure must not be captioned as unmeasured"
        );
    }

    #[test]
    fn says_plainly_when_the_estimate_is_a_guess() {
        let (mut status, state) = fixture();
        status.estimate = Estimate {
            hours: 20.0,
            source: EstimateSource::Default,
        };
        let v = view(&status, &state);
        assert_eq!(v.evidence, Evidence::Inferred);
        assert!(document(&v, Theme::System).contains("Not yet measured"));
    }

    #[test]
    fn theme_choice_reaches_the_document() {
        let (status, state) = fixture();
        let v = view(&status, &state);
        // Assert on the html tag, not the whole document: the stylesheet
        // mentions [data-theme=...] in its selectors no matter what is chosen.
        assert!(document(&v, Theme::Dark).contains(r#"<html lang="en" data-theme="dark">"#));
        assert!(document(&v, Theme::Light).contains(r#"<html lang="en" data-theme="light">"#));
        // System leaves the attribute off so prefers-color-scheme decides.
        assert!(document(&v, Theme::System).contains(r#"<html lang="en">"#));
    }

    #[test]
    fn both_themes_define_every_colour_token() {
        // A token defined only inside a media query leaves the panel borrowing
        // whatever the host paints when the other theme is chosen.
        for token in ["--page", "--text", "--glass", "--brand", "--muted"] {
            let light = PANEL_CSS.matches(&format!("{token}:")).count();
            assert!(
                light >= 3,
                "{token} should be defined for light, media-dark, and explicit dark"
            );
        }
    }

    #[test]
    fn the_scrollbar_is_hidden_but_scrolling_is_not() {
        assert!(PANEL_CSS.contains("scrollbar-width:none"));
        assert!(PANEL_CSS.contains("::-webkit-scrollbar"));
        assert!(PANEL_CSS.contains("overflow-y:auto"));
    }

    #[test]
    fn the_primary_button_is_never_the_status_colour() {
        // "Sign in" painted red because the session expired reads as delete.
        assert!(PANEL_CSS.contains("button.primary{background:var(--brand)"));
    }

    #[test]
    fn escapes_values_that_come_from_the_environment() {
        let (mut status, state) = fixture();
        status.config.as_mut().unwrap().name = "<img src=x onerror=alert(1)>".into();
        let page = document(&view(&status, &state), Theme::System);
        assert!(!page.contains("<img src=x"));
        assert!(page.contains("&lt;img"));
    }

    #[test]
    fn a_signed_out_panel_does_not_show_a_countdown() {
        let (mut status, state) = fixture();
        status.auth = AuthState::Expired;
        let v = view(&status, &state);
        assert_eq!(v.headline, "Signed out");
        assert!(v.rows.iter().all(|(k, _)| k != "Est. re-auth"));
    }

    #[test]
    fn gaps_are_computed_between_consecutive_logins() {
        let (status, state) = fixture();
        let v = view(&status, &state);
        assert_eq!(v.logins.len(), 3);
        assert_eq!(v.logins[0].2, "+16.0h");
        assert_eq!(v.logins[2].2, "");
    }

    #[test]
    fn one_sample_draws_no_sparkline() {
        let (status, mut state) = fixture();
        state.samples = vec![16.0];
        assert!(!body(&view(&status, &state)).contains("class=\"spark\""));
    }

    #[test]
    fn the_refresh_script_is_valid_javascript_literals() {
        let (status, state) = fixture();
        let s = refresh_script(&view(&status, &state), Theme::Dark);
        assert!(s.contains("setAttribute('data-theme', \"dark\")"));
        assert!(s.contains("document.body.innerHTML = \""));
    }
}
