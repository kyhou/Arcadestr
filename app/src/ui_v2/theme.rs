//! Stitch-inspired design tokens and utility classes for UI v2.

pub const UI_V2_STYLES: &str = r#"
@import url('https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@300;400;500;600;700&family=Inter:wght@300;400;500;600;700&display=swap');
@import url('https://fonts.googleapis.com/css2?family=Material+Symbols+Outlined:wght,FILL@100..700,0..1&display=swap');

:root {
  --v2-font-display: 'Space Grotesk', 'Inter', system-ui, sans-serif;
  --v2-font-body: 'Inter', 'Space Grotesk', system-ui, sans-serif;

  --v2-background: #0a0e14;
  --v2-on-background: #f1f3fc;

  --v2-surface-lowest: #000000;
  --v2-surface-low: #0f141a;
  --v2-surface: #151a21;
  --v2-surface-high: #1b2028;
  --v2-surface-highest: #20262f;
  --v2-surface-bright: #262c36;

  --v2-primary: #b6a0ff;
  --v2-primary-dim: #7e51ff;
  --v2-on-primary: #340090;

  --v2-secondary: #00d2fd;
  --v2-secondary-dim: #00c3eb;
  --v2-on-secondary: #004352;

  --v2-tertiary: #ff96bb;
  --v2-on-tertiary: #690939;

  --v2-outline: #72757d;
  --v2-outline-ghost: rgba(68, 72, 79, 0.15);
  --v2-on-surface-variant: #a8abb3;

  --v2-danger: #ff6e84;
  --v2-success: #00d2fd;

  --v2-radius-sm: 0.25rem;
  --v2-radius-md: 0.75rem;
  --v2-radius-lg: 1rem;
  --v2-radius-xl: 1.5rem;
  --v2-radius-full: 9999px;

  --v2-space-1: 0.25rem;
  --v2-space-2: 0.5rem;
  --v2-space-3: 0.75rem;
  --v2-space-4: 1rem;
  --v2-space-5: 1.5rem;
  --v2-space-6: 2rem;
  --v2-space-7: 3rem;

  --v2-shadow-ambient: 0 20px 40px rgba(0, 0, 0, 0.4);
}

* {
  box-sizing: border-box;
}

.material-symbols-outlined {
  font-family: 'Material Symbols Outlined', sans-serif;
  font-variation-settings: 'FILL' 0, 'wght' 400, 'GRAD' 0, 'opsz' 24;
}

.material-symbols-outlined.v2-icon-24 {
  font-size: 24px;
  line-height: 24px;
}

.material-symbols-outlined.v2-icon-16 {
  font-size: 16px;
  line-height: 24px;
}

.material-symbols-outlined.v2-icon-14 {
  font-size: 14px;
  line-height: 20px;
}

.material-symbols-outlined.v2-icon-12 {
  font-size: 12px;
  line-height: 16px;
}

.material-symbols-outlined.v2-icon-30 {
  font-size: 30px;
  line-height: 36px;
}

.glass-panel {
  background: rgba(27, 32, 40, 0.6);
  backdrop-filter: blur(24px);
  -webkit-backdrop-filter: blur(24px);
  border: 1px solid rgba(68, 72, 79, 0.15);
}

.v2-app {
  min-height: 100vh;
  color: var(--v2-on-background);
  background:
    radial-gradient(circle at 10% 0%, rgba(126, 81, 255, 0.18), transparent 40%),
    radial-gradient(circle at 90% 5%, rgba(0, 210, 253, 0.16), transparent 45%),
    var(--v2-background);
  font-family: var(--v2-font-body);
}

.v2-shell-grid {
  display: block;
}

.v2-brand-gradient {
  background: linear-gradient(120deg, var(--v2-primary) 0%, var(--v2-primary-dim) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.v2-top-links {
  display: inline-flex;
  align-items: center;
  gap: var(--v2-space-3);
}

.v2-top-link {
  color: rgba(241, 243, 252, 0.7);
  text-decoration: none;
  font-family: var(--v2-font-display);
  font-size: 1.02rem;
  line-height: 1.1;
}

.v2-top-link-active {
  color: rgba(241, 243, 252, 0.95);
}

.v2-sidebar {
  position: fixed;
  top: 68px;
  left: 0;
  width: 256px;
  height: calc(100vh - 68px);
  padding: var(--v2-space-4);
  z-index: 40;
  background: rgba(15, 20, 26, 0.6);
  backdrop-filter: blur(24px);
  border-right: 1px solid rgba(68, 72, 79, 0.15);
  box-shadow: 20px 0 40px rgba(0, 0, 0, 0.4);
}

.v2-sidebar h3 {
  margin: 0 0 var(--v2-space-4) 0;
}

.v2-sidebar-profile {
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
  padding: 0 var(--v2-space-2);
  margin-bottom: var(--v2-space-4);
}

.v2-sidebar-avatar-ring {
  width: 48px;
  height: 48px;
  border-radius: var(--v2-radius-full);
  padding: 2px;
  background: linear-gradient(135deg, var(--v2-primary), var(--v2-secondary));
}

.v2-sidebar-avatar-ring img {
  width: 100%;
  height: 100%;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-sidebar-login-avatar {
  width: 40px;
  height: 40px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-sidebar-zaps {
  margin: 0;
  color: var(--v2-tertiary);
  font-size: 0.72rem;
  font-weight: 700;
}

.v2-sidebar h3 {
  margin: 0 0 var(--v2-space-4) 0;
}

.v2-sidebar-nav {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-nav-item {
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
  text-align: left;
  padding: var(--v2-space-3) var(--v2-space-4);
  border-radius: var(--v2-radius-md);
  border: 1px solid transparent;
  background: transparent;
  color: rgba(241, 243, 252, 0.5);
  cursor: pointer;
  transition: transform 200ms ease, background 200ms ease, color 200ms ease;
}

.v2-nav-item:hover {
  transform: translateX(4px);
  background: rgba(38, 44, 54, 0.3);
  color: var(--v2-on-background);
}

.v2-nav-item-icon {
  width: 1.2rem;
  height: 1.2rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--v2-radius-full);
  background: rgba(32, 38, 47, 0.7);
  font-size: 0.72rem;
}

.v2-nav-item-icon-active {
  font-variation-settings: 'FILL' 1, 'wght' 500, 'GRAD' 0, 'opsz' 24;
}

.v2-nav-item-active {
  background: var(--v2-surface-high);
  color: var(--v2-primary);
  border-color: rgba(68, 72, 79, 0.15);
}

.v2-main-column {
  margin-left: 256px;
  padding-top: 68px;
  min-height: 100vh;
}

.v2-topbar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  z-index: 50;
  height: 68px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0 1.6rem;
  background: linear-gradient(90deg, #03050b 0%, #0a1220 56%, #0a2231 100%);
  border-bottom: 1px solid rgba(68, 72, 79, 0.18);
}

.v2-topbar-left {
  display: flex;
  align-items: center;
  gap: 1.6rem;
  min-width: 0;
}

.v2-topbar-search {
  width: 100%;
  height: 42px;
  padding: 0;
  border-radius: 0;
  border: none;
  background: transparent;
}

.v2-topbar-search-wrap {
  width: min(340px, 34vw);
  height: 42px;
  display: flex;
  align-items: center;
  gap: 0.75rem;
  border-radius: 12px;
  background: rgba(32, 38, 47, 0.78);
  padding: 0 1.05rem;
}

.v2-topbar-search-wrap .material-symbols-outlined {
  position: static;
  transform: none;
  color: var(--v2-on-surface-variant);
  font-size: 0.95rem;
  pointer-events: none;
  z-index: 1;
}

.v2-topbar-right {
  display: flex;
  gap: 0.85rem;
  align-items: center;
}

.v2-topbar-right .material-symbols-outlined {
  font-size: 1rem;
}

.v2-relay-pill,
.v2-user-pill {
  display: inline-flex;
  align-items: center;
   gap: 0.35rem;
  padding: var(--v2-space-2) var(--v2-space-3);
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
}

.v2-relay-pill strong {
  font-size: 0.9rem;
}

.v2-icon-btn {
  border: 1px solid transparent;
  border-radius: var(--v2-radius-full);
  background: transparent;
  color: rgba(241, 243, 252, 0.82);
  width: 2rem;
  height: 2rem;
}

.v2-icon-btn:hover {
  color: #ffffff;
  background: rgba(38, 44, 54, 0.35);
}

.v2-topbar-avatar {
  width: 40px;
  height: 40px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
  border: 2px solid rgba(126, 81, 255, 0.35);
  padding: 2px;
}

.v2-connection-pill {
  display: inline-flex;
  align-items: center;
  padding: var(--v2-space-2) var(--v2-space-3);
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
  color: var(--v2-on-surface-variant);
}

.v2-connection-ok {
  color: var(--v2-secondary);
}

.v2-connection-pending {
  color: var(--v2-tertiary);
}

.v2-connection-failed {
  color: var(--v2-danger);
}

.v2-content {
  max-width: 1600px;
  margin: 0 auto;
  padding: 2rem;
}

.v2-section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-3);
  margin-bottom: var(--v2-space-3);
}

.v2-sidebar-footer {
  margin-top: var(--v2-space-4);
  display: grid;
  gap: var(--v2-space-2);
}

.v2-sidebar-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-2);
}

.v2-connect-btn {
  margin-top: var(--v2-space-4);
  width: 100%;
  padding: var(--v2-space-3) var(--v2-space-4);
}

.v2-store-front {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-store-categories-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-store-category-tile {
  min-height: 96px;
  border: none;
  border-radius: var(--v2-radius-lg);
  color: var(--v2-on-background);
  font-family: var(--v2-font-display);
  font-weight: 700;
  letter-spacing: 0.04em;
}

.v2-store-category-primary { background: rgba(182, 160, 255, 0.2); }
.v2-store-category-secondary { background: rgba(0, 210, 253, 0.2); }
.v2-store-category-tertiary { background: rgba(255, 150, 187, 0.2); }
.v2-store-category-neutral { background: var(--v2-surface-highest); }

.v2-store-front-hero {
  min-height: 500px;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  padding: var(--v2-space-5);
}

.v2-store-kicker {
  margin: 0;
  color: var(--v2-tertiary);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
}

.v2-store-front-content {
  display: block;
}

.v2-store-layout-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: 2rem;
}

.v2-hero-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 280px;
  gap: var(--v2-space-5);
}

.v2-hero-title {
  margin: 0 0 var(--v2-space-3) 0;
  font-size: clamp(2.4rem, 4.4vw, 3.4rem);
  text-transform: uppercase;
  letter-spacing: 0.02em;
}

.v2-hero-description {
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.5;
}

.v2-hero-actions {
  margin-top: var(--v2-space-4);
  display: flex;
  gap: var(--v2-space-2);
}

.v2-hero-metrics {
  display: grid;
  gap: var(--v2-space-3);
}

.v2-hero-media-row {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--v2-space-2);
}

.v2-hero-media-row img {
  width: 100%;
  height: 84px;
  object-fit: cover;
  border-radius: var(--v2-radius-md);
}

.v2-metric-card {
  background: var(--v2-surface-highest);
  border-radius: var(--v2-radius-lg);
  padding: var(--v2-space-3);
  display: grid;
  gap: var(--v2-space-1);
}

.v2-category-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--v2-space-2);
}

.v2-category-chips span,
.v2-chip {
  display: inline-flex;
  align-items: center;
  padding: var(--v2-space-1) var(--v2-space-2);
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
  color: var(--v2-on-surface-variant);
  font-size: 0.85rem;
}

.v2-trending-block,
.v2-live-notes-block {
  padding: var(--v2-space-4);
  border-radius: var(--v2-radius-xl);
}

.v2-game-card-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-game-card {
  border: 1px solid transparent;
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-highest);
  color: var(--v2-on-background);
  text-align: left;
  padding: var(--v2-space-3);
  cursor: pointer;
  display: grid;
  gap: var(--v2-space-2);
}

.v2-game-card:hover {
  border-color: var(--v2-outline-ghost);
  background: var(--v2-surface-bright);
}

.v2-game-card-image {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  border-radius: var(--v2-radius-md);
}

.v2-game-card h4 {
  margin: 0;
}

.v2-game-card p {
  margin: 0;
  color: var(--v2-on-surface-variant);
}

.v2-game-card-zaps {
  color: var(--v2-tertiary);
  font-size: 0.72rem;
  font-weight: 700;
}

.v2-game-card-subtitle {
  font-size: 0.72rem;
  color: var(--v2-on-surface-variant);
  font-style: italic;
}

.v2-game-card-footer {
  display: flex;
  justify-content: space-between;
  gap: var(--v2-space-2);
  align-items: end;
}

.v2-game-card-price-sats {
  margin: 0;
  font-size: 0.72rem;
  color: var(--v2-on-surface-variant);
  font-weight: 600;
}

.v2-game-card-price-usd {
  margin: 0;
  font-size: 0.84rem;
  font-weight: 700;
}

.v2-game-card-cta {
  background: var(--v2-secondary);
  color: var(--v2-on-secondary);
  border-radius: var(--v2-radius-md);
  padding: 0.5rem 0.8rem;
  font-size: 0.72rem;
  font-weight: 700;
}

.v2-live-note {
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-md);
  background: rgba(21, 26, 33, 0.7);
  margin-bottom: var(--v2-space-2);
}

.v2-live-note:last-child {
  margin-bottom: 0;
}

.v2-live-note p {
  margin: 0;
}

.v2-live-note-head {
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
  margin-bottom: 0.55rem;
}

.v2-live-note-head img {
  width: 40px;
  height: 40px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-live-note-meta {
  margin-top: 0.1rem !important;
  margin-bottom: 0 !important;
  color: var(--v2-on-surface-variant);
  font-size: 0.76rem;
}

.v2-live-note-actions {
  margin-top: var(--v2-space-2);
  display: flex;
  gap: var(--v2-space-3);
}

.v2-live-note-actions span {
  display: inline-flex;
  align-items: center;
  gap: 0.2rem;
  color: var(--v2-tertiary);
  font-size: 0.78rem;
  font-weight: 700;
}

.v2-live-note-actions .material-symbols-outlined {
  font-size: 0.92rem;
}

.v2-detail-wrap,
.v2-publish-wrap {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-detail-description-block {
  padding: var(--v2-space-5);
}

.v2-detail-description-block h2 {
  margin: 0 0 var(--v2-space-3) 0;
}

.v2-detail-description-block p {
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.6;
  max-width: 72ch;
}

.v2-detail-hero {
  min-height: 716px;
  padding: var(--v2-space-5);
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: var(--v2-space-4);
}

.v2-detail-title {
  margin: 0 0 var(--v2-space-3) 0;
}

.v2-detail-rating-row {
  display: flex;
  gap: var(--v2-space-2);
  margin-bottom: var(--v2-space-2);
  color: var(--v2-on-surface-variant);
  font-size: 0.85rem;
}

.v2-detail-tags {
  margin-top: var(--v2-space-3);
  display: flex;
  flex-wrap: wrap;
  gap: var(--v2-space-2);
}

.v2-detail-buy-panel {
  padding: var(--v2-space-4);
  display: grid;
  gap: var(--v2-space-2);
  align-content: start;
}

.v2-detail-price {
  font-size: 1.5rem;
  font-weight: 800;
}

.v2-detail-currently-playing {
  margin-top: var(--v2-space-3);
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
}

.v2-detail-currently-playing h4 {
  margin: 0 0 var(--v2-space-2) 0;
}

.v2-playing-row {
  display: flex;
  justify-content: space-between;
  margin-bottom: var(--v2-space-1);
  font-size: 0.85rem;
}

.v2-detail-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: var(--v2-space-4);
}

.v2-detail-feed,
.v2-detail-specs,
.v2-detail-transaction-wrap {
  padding: var(--v2-space-4);
}

.v2-detail-gallery-grid {
  display: grid;
  grid-template-columns: 2fr 1fr 1fr;
  gap: var(--v2-space-2);
  margin-bottom: var(--v2-space-4);
}

.v2-detail-gallery-grid img {
  width: 100%;
  height: 100%;
  min-height: 140px;
  object-fit: cover;
  border-radius: var(--v2-radius-md);
}

.v2-spec-grid {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--v2-space-2) var(--v2-space-3);
}

.v2-spec-grid span:nth-child(odd) {
  color: var(--v2-on-surface-variant);
}

.v2-detail-note-card {
  margin-top: var(--v2-space-2);
}

.v2-library-grid,
.v2-social-grid {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-tab-row {
  display: flex;
  gap: var(--v2-space-2);
  margin-top: var(--v2-space-3);
}

.v2-tab {
  border: 1px solid transparent;
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
  color: var(--v2-on-surface-variant);
  padding: var(--v2-space-1) var(--v2-space-3);
}

.v2-tab.active {
  color: var(--v2-on-background);
  border-color: var(--v2-outline-ghost);
}

.v2-library-layout-grid,
.v2-social-layout-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 360px;
  gap: var(--v2-space-4);
}

.v2-library-card-list,
.v2-social-main {
  padding: var(--v2-space-4);
}

.v2-library-main-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-library-feature-card {
  grid-column: span 2;
  position: relative;
  border-radius: var(--v2-radius-xl);
  overflow: hidden;
  min-height: 380px;
}

.v2-library-feature-card img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.v2-library-feature-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: var(--v2-space-3);
  padding: var(--v2-space-4);
  background: linear-gradient(to top, rgba(10, 14, 20, 0.9), rgba(10, 14, 20, 0.1));
}

.v2-library-media-card {
  background: var(--v2-surface-high);
  border-radius: var(--v2-radius-lg);
  overflow: hidden;
}

.v2-library-media-card img {
  width: 100%;
  aspect-ratio: 16 / 10;
  object-fit: cover;
}

.v2-library-media-copy {
  padding: var(--v2-space-3);
}

.v2-library-media-copy h4 {
  margin: 0;
}

.v2-library-card-row {
  background: var(--v2-surface-highest);
  border-radius: var(--v2-radius-lg);
  padding: var(--v2-space-3);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-3);
  margin-bottom: var(--v2-space-2);
}

.v2-library-card-row h4 {
  margin: 0;
}

.v2-library-side-grid,
.v2-social-side {
  display: grid;
  gap: var(--v2-space-3);
  align-content: start;
}

.v2-identity-card,
.v2-notes-card,
.v2-social-side-card {
  padding: var(--v2-space-4);
}

.v2-friends-card {
  padding: var(--v2-space-4);
}

.v2-friends-row {
  display: flex;
  gap: var(--v2-space-2);
  flex-wrap: wrap;
}

.v2-friends-row img {
  width: 40px;
  height: 40px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-friends-row .v2-btn-ghost {
  width: 40px;
  height: 40px;
  border-radius: var(--v2-radius-full);
}

.v2-stat-line {
  display: flex;
  justify-content: space-between;
  margin-top: var(--v2-space-2);
}

.v2-composer-row {
  margin-top: var(--v2-space-3);
  display: flex;
  gap: var(--v2-space-2);
}

.v2-chip-column {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-trending-list {
  display: grid;
  gap: var(--v2-space-3);
}

.v2-trending-item strong {
  display: block;
  font-size: 1rem;
}

.v2-library-hero,
.v2-social-hero {
  padding: var(--v2-space-5);
}

.v2-library-hero {
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: var(--v2-space-4);
}

.v2-library-hero h1 {
  margin: 0 0 var(--v2-space-1) 0;
  font-size: 2.4rem;
}

.v2-library-hero p {
  margin: 0;
  color: var(--v2-on-surface-variant);
}

.v2-library-tabs {
  background: var(--v2-surface-low);
  padding: 0.25rem;
  border-radius: var(--v2-radius-md);
  margin-top: 0;
}

.v2-library-card,
.v2-social-card {
  padding: var(--v2-space-4);
}

.v2-social-hero {
  padding: var(--v2-space-4);
  background: transparent;
  border-radius: 0;
}

.v2-social-composer-card {
  padding: var(--v2-space-4);
  border-radius: var(--v2-radius-xl);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-social-composer-head {
  display: flex;
  gap: var(--v2-space-3);
}

.v2-social-composer-avatar {
  width: 48px;
  height: 48px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-social-composer-text {
  width: 100%;
  min-height: 96px;
  resize: none;
  background: transparent;
  color: var(--v2-on-background);
  border: none;
  outline: none;
}

.v2-social-composer-text::placeholder {
  color: rgba(168, 171, 179, 0.7);
}

.v2-social-composer-actions {
  margin-top: var(--v2-space-3);
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-3);
}

.v2-social-composer-tools {
  display: flex;
  gap: var(--v2-space-2);
}

.v2-social-hero-media {
  width: 100%;
  height: 220px;
  object-fit: cover;
  border-radius: var(--v2-radius-lg);
  margin-bottom: var(--v2-space-3);
}

.v2-social-thumb-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--v2-space-2);
  margin: var(--v2-space-3) 0;
}

.v2-social-thumb-row img {
  width: 100%;
  height: 128px;
  object-fit: cover;
  border-radius: var(--v2-radius-md);
}

.v2-suggest-item {
  display: flex;
  gap: var(--v2-space-2);
  align-items: flex-start;
  margin-bottom: var(--v2-space-2);
}

.v2-suggest-item img {
  width: 56px;
  height: 56px;
  border-radius: var(--v2-radius-md);
  object-fit: cover;
}

.v2-suggest-item strong {
  display: block;
}

.v2-social-card h3 {
  margin: 0 0 var(--v2-space-2) 0;
}

.v2-zaps-card {
  background: linear-gradient(160deg, rgba(255, 150, 187, 0.1), rgba(21, 26, 33, 0.7));
  border: 1px solid rgba(255, 150, 187, 0.2);
}

.v2-zaps-card h3 {
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
}

.v2-zaps-card h3::before {
  content: "bolt";
  font-family: 'Material Symbols Outlined', sans-serif;
  color: var(--v2-tertiary);
}

.v2-zap-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-2);
  margin-bottom: var(--v2-space-2);
  font-size: 0.78rem;
}

.v2-zap-row strong {
  font-size: 0.62rem;
  letter-spacing: 0.06em;
  color: var(--v2-tertiary);
}

.v2-social-feed-header {
  margin-bottom: var(--v2-space-4);
}

.v2-social-feed-header h2 {
  margin: 0;
  font-size: 2rem;
}

.v2-social-meta {
  margin: 0 0 var(--v2-space-2) 0;
  color: var(--v2-on-surface-variant);
  font-size: 0.9rem;
}

.v2-social-actions {
  display: flex;
  gap: var(--v2-space-3);
  color: var(--v2-tertiary);
  font-weight: 700;
}

.v2-login-wrap {
  min-height: 100vh;
  padding: 0;
}

.v2-login-wrap .v2-main-column {
  display: flex;
  align-items: center;
  justify-content: center;
}

.v2-login-wrap .v2-content {
  position: relative;
  width: 100%;
}

.v2-login-glow {
  position: absolute;
  border-radius: var(--v2-radius-full);
  pointer-events: none;
  z-index: 0;
}

.v2-login-glow-left {
  width: 380px;
  height: 380px;
  left: -90px;
  top: 20%;
  background: rgba(182, 160, 255, 0.14);
  filter: blur(120px);
}

.v2-login-glow-right {
  width: 320px;
  height: 320px;
  right: -80px;
  bottom: 16%;
  background: rgba(0, 210, 253, 0.14);
  filter: blur(100px);
}

.v2-login-shell {
  width: min(860px, 100%);
  padding: var(--v2-space-6);
  position: relative;
  z-index: 1;
}

.v2-user-select-shell {
  width: min(960px, calc(100vw - 4rem));
}

.v2-user-select-header {
  text-align: center;
  margin-bottom: var(--v2-space-5);
}

.v2-user-select-header h1 {
  margin: 0 0 var(--v2-space-2) 0;
  font-size: clamp(2.2rem, 4.2vw, 3rem);
  letter-spacing: -0.02em;
}

.v2-user-select-header .v2-hero-description {
  max-width: 680px;
  margin: 0 auto;
}

.v2-add-account-shell {
  width: min(1100px, calc(100vw - 4rem));
}

.v2-login-content {
  margin-top: var(--v2-space-4);
}

.v2-add-account-body-grid {
  margin-top: var(--v2-space-4);
  display: grid;
  grid-template-columns: 7fr 5fr;
  gap: var(--v2-space-3);
  align-items: start;
}

.v2-add-account-form-panel {
  margin-top: 0;
  background: rgba(15, 20, 26, 0.45);
  border: 1px solid rgba(68, 72, 79, 0.18);
  border-radius: var(--v2-radius-lg);
  padding: var(--v2-space-4);
}

.v2-qr-connect-card {
  padding: var(--v2-space-4);
  border-radius: var(--v2-radius-xl);
}

.v2-qr-card-head {
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
  margin-bottom: var(--v2-space-3);
}

.v2-qr-card-head h3 {
  margin: 0;
  font-family: var(--v2-font-display);
}

.v2-qr-image-wrap {
  margin-bottom: var(--v2-space-3);
  display: flex;
  justify-content: center;
  background: white;
  border-radius: var(--v2-radius-lg);
  padding: var(--v2-space-3);
}

.v2-qr-image-wrap img {
  width: 176px;
  height: 176px;
  object-fit: cover;
}

.v2-dynamic-qr {
  width: 12rem;
  height: 12rem;
  overflow: hidden;
}

.v2-dynamic-qr svg {
  width: 100%;
  height: 100%;
  display: block;
}

.v2-manual-connect {
  margin-top: var(--v2-space-3);
  display: grid;
  grid-template-columns: 1fr auto;
  gap: var(--v2-space-2);
  align-items: center;
}

.v2-manual-connect .v2-input {
  font-size: 0.78rem;
}

.v2-user-card-list {
  margin-top: var(--v2-space-4);
  display: grid;
  gap: var(--v2-space-2);
  margin-bottom: var(--v2-space-4);
}

.v2-user-profile-grid {
  margin-top: var(--v2-space-4);
  margin-bottom: var(--v2-space-4);
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-user-profile-card {
  background: var(--v2-surface-high);
  border-radius: var(--v2-radius-xl);
  border: 1px solid transparent;
  padding: var(--v2-space-4);
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: var(--v2-space-2);
  color: var(--v2-on-background);
  transition: border-color 180ms ease, background 180ms ease, transform 180ms ease;
}

.v2-user-profile-card:hover {
  border-color: rgba(182, 160, 255, 0.35);
  background: var(--v2-surface-bright);
  transform: translateY(-2px);
}

.v2-user-profile-card h3 {
  margin: 0;
  font-family: var(--v2-font-display);
  font-size: 1.05rem;
}

.v2-user-profile-avatar-wrap {
  position: relative;
  width: 96px;
  height: 96px;
  border-radius: var(--v2-radius-full);
  padding: 2px;
  background: rgba(68, 72, 79, 0.4);
}

.v2-user-profile-avatar-active {
  background: linear-gradient(120deg, var(--v2-primary), var(--v2-secondary));
}

.v2-user-profile-avatar {
  width: 100%;
  height: 100%;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
}

.v2-user-profile-check {
  position: absolute;
  right: -4px;
  bottom: -4px;
  width: 24px;
  height: 24px;
  border-radius: var(--v2-radius-full);
  background: var(--v2-primary);
  color: #ffffff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 0.9rem;
}

.v2-user-profile-pill {
  margin-top: var(--v2-space-1);
  padding: 0.2rem 0.55rem;
  border-radius: var(--v2-radius-sm);
  font-size: 0.62rem;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  background: var(--v2-surface-lowest);
}

.v2-user-profile-pill-tertiary {
  color: var(--v2-tertiary);
}

.v2-user-profile-pill-secondary {
  color: var(--v2-secondary);
}

.v2-user-profile-pill-muted {
  color: var(--v2-on-surface-variant);
}

.v2-user-add-card {
  border: 2px dashed rgba(68, 72, 79, 0.35);
  border-radius: var(--v2-radius-xl);
  background: var(--v2-surface-low);
  color: var(--v2-on-surface-variant);
  padding: var(--v2-space-4);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: var(--v2-space-2);
}

.v2-user-add-card:hover {
  border-color: rgba(182, 160, 255, 0.5);
  color: var(--v2-on-background);
}

.v2-user-add-icon-wrap {
  width: 64px;
  height: 64px;
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
  display: flex;
  align-items: center;
  justify-content: center;
}

.v2-user-add-icon-wrap .material-symbols-outlined {
  color: var(--v2-primary);
  font-size: 2rem;
}

.v2-user-empty-state {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-user-card {
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: rgba(21, 26, 33, 0.6);
  color: var(--v2-on-background);
  text-align: left;
  padding: var(--v2-space-3);
}

.v2-user-card strong {
  display: block;
}

.v2-user-card-avatar {
  width: 56px;
  height: 56px;
  border-radius: var(--v2-radius-full);
  object-fit: cover;
  border: 1px solid rgba(68, 72, 79, 0.25);
}

.v2-login-actions {
  margin-top: var(--v2-space-4);
  display: grid;
  gap: var(--v2-space-2);
  justify-items: center;
}

.v2-connect-main-btn {
  min-width: 280px;
  min-height: 56px;
  padding: 0.9rem 2.4rem;
  font-size: 1.08rem;
  font-weight: 700;
  border-radius: 0.6rem;
  box-shadow: 0 12px 26px rgba(126, 81, 255, 0.26);
}

.v2-create-identity-link {
  border: none;
  background: transparent;
  color: var(--v2-on-surface-variant);
  font-size: 0.9rem;
  font-weight: 500;
}

.v2-create-identity-link:hover {
  color: var(--v2-on-background);
}

.v2-create-identity-link::after {
  content: "";
  display: block;
  height: 1px;
  margin-top: 2px;
  background: rgba(182, 160, 255, 0.65);
}

.v2-user-select-footer {
  margin-top: var(--v2-space-5);
  display: flex;
  gap: var(--v2-space-3);
  justify-content: center;
  color: rgba(168, 171, 179, 0.7);
  font-size: 0.75rem;
}

.v2-method-grid {
  margin-top: var(--v2-space-4);
  display: grid;
  grid-template-columns: 7fr 5fr;
  grid-template-areas:
    "bunker qr"
    "nsec qr";
  gap: var(--v2-space-3);
}

.v2-method-card {
  padding: var(--v2-space-4);
  border-radius: var(--v2-radius-lg);
  background: rgba(21, 26, 33, 0.5);
  border: 1px solid rgba(68, 72, 79, 0.18);
}

.v2-method-card:nth-child(1) {
  grid-area: bunker;
}

.v2-method-card:nth-child(2) {
  grid-area: nsec;
}

.v2-method-card:nth-child(3) {
  grid-area: qr;
}

.v2-method-card h3 {
  margin: 0 0 var(--v2-space-2) 0;
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
}

.v2-account-footer {
  margin-top: var(--v2-space-5);
  padding-top: var(--v2-space-4);
  border-top: 1px solid var(--v2-outline-ghost);
  text-align: center;
  color: rgba(168, 171, 179, 0.7);
  font-size: 0.7rem;
}

.v2-settings-wrap {
  padding: var(--v2-space-4);
}

.v2-mobile-bottom-nav {
  position: fixed;
  left: 0;
  right: 0;
  bottom: 0;
  display: flex;
  justify-content: space-around;
  align-items: center;
  min-height: 60px;
  background: rgba(15, 20, 26, 0.9);
  border-top: 1px solid var(--v2-outline-ghost);
  backdrop-filter: blur(20px);
  z-index: 70;
}

.v2-mobile-nav-item {
  display: grid;
  place-items: center;
  gap: 0.1rem;
  border: none;
  background: transparent;
  color: var(--v2-on-surface-variant);
}

.v2-mobile-nav-item small {
  font-size: 0.65rem;
}

.v2-profile-grid {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-profile-hero,
.v2-profile-listings {
  padding: var(--v2-space-4);
}

.v2-profile-listings-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-3);
  margin-bottom: var(--v2-space-3);
}

.v2-profile-list {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-profile-list-item {
  display: flex;
  justify-content: space-between;
  background: var(--v2-surface-highest);
  padding: var(--v2-space-3);
}

.v2-display {
  font-family: var(--v2-font-display);
  letter-spacing: -0.02em;
}

.v2-panel {
  background: var(--v2-surface-high);
  border-radius: var(--v2-radius-xl);
}

.v2-panel-glass {
  background: rgba(27, 32, 40, 0.6);
  border: 1px solid var(--v2-outline-ghost);
  backdrop-filter: blur(24px);
  box-shadow: var(--v2-shadow-ambient);
  border-radius: var(--v2-radius-xl);
}

.v2-btn-primary {
  border: none;
  border-radius: var(--v2-radius-md);
  background: linear-gradient(120deg, var(--v2-primary) 0%, var(--v2-primary-dim) 100%);
  color: var(--v2-on-primary);
  font-weight: 700;
  cursor: pointer;
}

.v2-btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.v2-btn-secondary {
  border: none;
  border-radius: var(--v2-radius-md);
  background: var(--v2-secondary);
  color: var(--v2-on-secondary);
  font-weight: 700;
  cursor: pointer;
}

.v2-btn-ghost {
  border: 1px solid transparent;
  border-radius: var(--v2-radius-md);
  background: transparent;
  color: var(--v2-on-background);
  cursor: pointer;
}

.v2-btn-ghost:hover {
  background: rgba(32, 38, 47, 0.3);
}

.v2-input {
  width: 100%;
  border: 1px solid transparent;
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-highest);
  color: var(--v2-on-background);
  padding: var(--v2-space-3) var(--v2-space-4);
}

.v2-input::placeholder {
  color: var(--v2-on-surface-variant);
}

.v2-input.v2-topbar-search {
  padding: 0;
  border: none;
  background: transparent;
}

.v2-input.v2-topbar-search::placeholder {
  color: rgba(168, 171, 179, 0.9);
  text-indent: 0.4rem;
}

.v2-input:focus,
.v2-input:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px rgba(0, 195, 235, 0.4);
  border-color: var(--v2-secondary-dim);
}

@media (max-width: 960px) {
  .v2-sidebar {
    display: none;
  }

  .v2-main-column {
    margin-left: 0;
    padding-top: 80px;
    padding-bottom: 76px;
  }

  .v2-store-layout-grid,
  .v2-store-categories-grid,
  .v2-hero-grid,
  .v2-detail-hero,
  .v2-detail-grid,
  .v2-user-profile-grid,
  .v2-library-main-grid,
  .v2-library-layout-grid,
  .v2-social-layout-grid,
  .v2-game-card-grid,
  .v2-detail-gallery-grid,
  .v2-method-grid,
  .v2-add-account-body-grid {
    grid-template-columns: 1fr;
  }

  .v2-hero-actions,
  .v2-composer-row {
    flex-direction: column;
  }

  .v2-topbar-left {
    flex-direction: column;
    align-items: stretch;
  }

  .v2-library-hero {
    flex-direction: column;
    align-items: flex-start;
  }

  .v2-user-select-footer {
    flex-wrap: wrap;
  }

  .v2-topbar-search {
    width: 100%;
  }

  .v2-hide-mobile {
    display: none !important;
  }
}

@media (min-width: 961px) {
  .v2-hide-desktop {
    display: none !important;
  }
}
"#;
