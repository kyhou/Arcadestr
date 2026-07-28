//! Arcadestr Noir design tokens and compatibility classes for UI v2.

pub const UI_V2_STYLES: &str = r#"
:root {
  --v2-font-display: 'Space Grotesk', 'Inter', ui-sans-serif, system-ui, sans-serif;
  --v2-font-body: 'Inter', ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;

  --v2-background: oklch(0.16 0.02 270);
  --v2-on-background: oklch(0.96 0.01 270);

  --v2-surface-lowest: oklch(0.13 0.015 270);
  --v2-surface-low: oklch(0.19 0.02 270);
  --v2-surface: oklch(0.215 0.025 273);
  --v2-surface-high: oklch(0.24 0.03 275);
  --v2-surface-highest: oklch(0.265 0.032 275);
  --v2-surface-bright: oklch(0.29 0.035 275);

  --v2-primary: oklch(0.78 0.16 300);
  --v2-primary-dim: oklch(0.6 0.24 295);
  --v2-on-primary: oklch(0.16 0.05 270);

  --v2-secondary: oklch(0.82 0.16 220);
  --v2-secondary-dim: oklch(0.72 0.15 220);
  --v2-on-secondary: oklch(0.16 0.05 270);

  --v2-tertiary: oklch(0.78 0.14 355);
  --v2-on-tertiary: oklch(0.16 0.05 270);

  --v2-outline: oklch(0.58 0.025 270);
  --v2-outline-ghost: oklch(1 0 0 / 8%);
  --v2-on-surface-variant: oklch(0.68 0.03 270);

  --v2-danger: oklch(0.66 0.22 20);
  --v2-success: oklch(0.75 0.15 160);

  --v2-radius-sm: 0.25rem;
  --v2-radius-md: 0.75rem;
  --v2-radius-lg: 1rem;
  --v2-radius-xl: 1.25rem;
  --v2-radius-full: 9999px;

  --v2-space-1: 0.25rem;
  --v2-space-2: 0.5rem;
  --v2-space-3: 0.75rem;
  --v2-space-4: 1rem;
  --v2-space-5: 1.5rem;
  --v2-space-6: 2rem;
  --v2-space-7: 3rem;

  --v2-shadow-ambient: 0 20px 40px rgba(0, 0, 0, 0.4);
  --v2-shadow-glow-primary: 0 0 32px oklch(0.78 0.16 300 / 35%);
  --v2-gradient-primary: linear-gradient(135deg, var(--v2-primary), var(--v2-primary-dim));
  --v2-gradient-hero: linear-gradient(180deg, transparent 0%, oklch(0.1 0.02 270 / 85%) 100%);
}

* {
  box-sizing: border-box;
}

select {
  color-scheme: dark;
  accent-color: var(--v2-primary);
  -webkit-appearance: none;
  appearance: none;
  background-color: var(--v2-surface-highest) !important;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 20' fill='%23a8abb3'%3E%3Cpath d='M5.5 7.5 10 12l4.5-4.5z'/%3E%3C/svg%3E");
  background-position: right 0.75rem center;
  background-repeat: no-repeat;
  background-size: 1rem;
  border: 0;
  padding-right: 2.5rem !important;
}

select option {
  color: var(--v2-on-background);
  background-color: var(--v2-surface-highest);
}

select option:disabled {
  color: var(--v2-on-surface-variant);
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
  background: oklch(0.24 0.03 275 / 65%);
  backdrop-filter: blur(24px);
  -webkit-backdrop-filter: blur(24px);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-app {
  min-height: 100vh;
  color: var(--v2-on-background);
  background: var(--v2-background);
  font-family: var(--v2-font-body);
  font-feature-settings: 'cv02', 'cv03', 'cv04', 'cv11';
}

.v2-shell-grid {
  display: block;
}

.v2-brand-gradient {
  background: var(--v2-gradient-primary);
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

.v2-detail-wrap {
  display: grid;
  gap: var(--v2-space-4);
  max-width: 1440px;
  margin: 0 auto;
}

.v2-publish-wrap {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-detail-back {
  width: fit-content;
  display: inline-flex;
  align-items: center;
  gap: var(--v2-space-1);
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
  padding: var(--v2-space-5);
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(220px, 0.55fr) 320px;
  align-items: center;
  gap: var(--v2-space-5);
  overflow: hidden;
  background:
    radial-gradient(circle at 70% 15%, rgba(201, 142, 255, 0.13), transparent 42%),
    linear-gradient(145deg, var(--v2-surface-low), var(--v2-surface));
}

.v2-detail-hero-copy {
  align-self: end;
}

.v2-detail-title {
  margin: 0 0 var(--v2-space-3) 0;
  max-width: 14ch;
  font-size: clamp(2.4rem, 5vw, 5.2rem);
  line-height: 0.95;
}

.v2-detail-cover-frame {
  aspect-ratio: 3 / 4;
  border-radius: var(--v2-radius-xl);
  overflow: hidden;
  box-shadow: 0 24px 70px rgba(0, 0, 0, 0.42);
  transform: rotate(1.5deg);
}

.v2-detail-cover-frame img {
  width: 100%;
  height: 100%;
  object-fit: cover;
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
  min-width: 0;
  max-width: 100%;
}

.v2-detail-buy-panel .v2-btn-primary,
.v2-detail-buy-panel .v2-btn-secondary,
.v2-detail-buy-panel .v2-btn-ghost {
  width: 100%;
  max-width: 100%;
  min-width: 0;
  min-height: 44px;
  padding: var(--v2-space-2) var(--v2-space-3);
  overflow-wrap: anywhere;
  white-space: normal;
}

.v2-detail-buy-panel > * {
  min-width: 0;
}

.v2-detail-buy-panel .v2-social-meta {
  max-width: 100%;
  min-width: 0;
  overflow-wrap: anywhere;
}

.v2-detail-price {
  margin-bottom: var(--v2-space-2);
  font-size: 1.8rem;
  font-weight: 800;
}

.v2-detail-status {
  margin: var(--v2-space-1) 0 0;
  color: var(--v2-secondary);
  font-size: 0.82rem;
  font-weight: 700;
}

.v2-detail-alert {
  margin: var(--v2-space-1) 0 0;
  padding: var(--v2-space-2);
  border-radius: var(--v2-radius-md);
  font-size: 0.82rem;
  overflow-wrap: anywhere;
}

.v2-detail-alert-error {
  color: var(--v2-error);
  background: rgba(255, 91, 116, 0.1);
}

.v2-detail-alert-success {
  color: var(--v2-success);
  background: rgba(83, 219, 151, 0.1);
}

.v2-detail-confirm,
.v2-detail-invoice {
  display: grid;
  gap: var(--v2-space-2);
  padding: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-low);
}

.v2-detail-invoice code {
  max-height: 84px;
  overflow: auto;
  padding: var(--v2-space-2);
  border-radius: var(--v2-radius-sm);
  background: var(--v2-surface-lowest);
  color: var(--v2-on-surface-variant);
  overflow-wrap: anywhere;
  white-space: normal;
}

.v2-detail-action-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--v2-space-2);
}

.v2-detail-buy-meta {
  display: grid;
  gap: 0.35rem;
  margin-top: var(--v2-space-2);
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
  color: var(--v2-on-surface-variant);
  font-size: 0.76rem;
}

.v2-detail-media {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: minmax(260px, 42%);
  gap: var(--v2-space-3);
  overflow-x: auto;
  padding-bottom: var(--v2-space-2);
  scroll-snap-type: x mandatory;
}

.v2-detail-media img {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  border-radius: var(--v2-radius-lg);
  scroll-snap-align: start;
}

.v2-detail-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: var(--v2-space-4);
  align-items: start;
}

.v2-detail-main-column {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-detail-campaign-list {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-detail-campaign-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--v2-space-3);
  padding: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-low);
}

.v2-detail-campaign-card p {
  margin: 0.25rem 0 0;
}

.v2-detail-campaign-card > div {
  min-width: 0;
  overflow-wrap: anywhere;
}

.v2-spec-grid {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--v2-space-2) var(--v2-space-3);
}

.v2-spec-grid span:nth-child(odd) {
  color: var(--v2-on-surface-variant);
}

.v2-detail-technical-value {
  min-width: 0;
  overflow-wrap: anywhere;
}

.v2-detail-seller-card {
  position: sticky;
  top: calc(var(--v2-space-4) + 76px);
  padding: var(--v2-space-4);
}

.v2-detail-seller-identity {
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
  margin-top: var(--v2-space-3);
}

.v2-detail-seller-identity h3 {
  margin: 0;
}

.v2-detail-seller-avatar {
  width: 52px;
  height: 52px;
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
  background: var(--v2-primary-container);
  color: var(--v2-on-primary-container);
  font-weight: 800;
}

.v2-detail-seller-about {
  margin: var(--v2-space-3) 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.55;
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

.v2-library-wrap {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-library-collection-label {
  display: inline-flex;
  align-items: center;
  min-height: 38px;
  padding: var(--v2-space-1) var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-high);
  color: var(--v2-secondary);
  font-size: 0.75rem;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.v2-library-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 300px;
  gap: var(--v2-space-4);
  align-items: start;
}

.v2-library-main {
  min-width: 0;
  display: grid;
  gap: var(--v2-space-3);
}

.v2-library-controls {
  display: grid;
  grid-template-columns: minmax(240px, 1fr) auto auto;
  align-items: end;
  gap: var(--v2-space-3);
  padding: var(--v2-space-3);
}

.v2-library-search-field,
.v2-library-filter-group {
  min-width: 0;
}

.v2-library-search-field label,
.v2-library-filter-group legend {
  display: block;
  margin-bottom: var(--v2-space-1);
  color: var(--v2-on-surface-variant);
  font-size: 0.75rem;
  font-weight: 700;
}

.v2-library-search-field > div {
  position: relative;
}

.v2-library-search-field .material-symbols-outlined {
  position: absolute;
  left: var(--v2-space-3);
  top: 50%;
  z-index: 1;
  color: var(--v2-on-surface-variant);
  transform: translateY(-50%);
}

.v2-library-search-field .v2-input {
  padding-left: 2.8rem;
}

.v2-library-filter-group {
  display: flex;
  flex-wrap: wrap;
  gap: var(--v2-space-1);
  margin: 0;
  padding: 0;
  border: 0;
}

.v2-library-filter-group legend {
  width: 100%;
}

.v2-library-controls > .v2-btn-secondary,
.v2-library-state-card .v2-btn-secondary,
.v2-library-card-body > .v2-btn-primary {
  min-height: 44px;
  padding: var(--v2-space-2) var(--v2-space-3);
}

.v2-library-result-count {
  margin: 0;
  color: var(--v2-on-surface-variant);
  font-size: 0.8rem;
}

.v2-library-notice {
  display: grid;
  gap: 0.3rem;
  padding: var(--v2-space-3);
  border: 1px solid rgba(255, 190, 92, 0.24);
  border-radius: var(--v2-radius-md);
  background: rgba(255, 190, 92, 0.07);
  color: var(--v2-on-surface-variant);
  font-size: 0.84rem;
}

.v2-library-notice strong {
  color: var(--v2-on-background);
}

.v2-library-state-card {
  min-height: 260px;
  display: grid;
  place-items: center;
  align-content: center;
  gap: var(--v2-space-2);
  padding: var(--v2-space-5);
  text-align: center;
}

.v2-library-state-card h2,
.v2-library-state-card p {
  margin: 0;
}

.v2-library-state-card p {
  max-width: 60ch;
  color: var(--v2-on-surface-variant);
}

.v2-library-state-card > .material-symbols-outlined {
  font-size: 2.4rem;
  color: var(--v2-secondary);
}

.v2-library-card-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-library-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: var(--v2-surface-high);
}

.v2-library-card-media {
  position: relative;
  aspect-ratio: 16 / 9;
  overflow: hidden;
  background: var(--v2-surface-lowest);
}

.v2-library-card-media::after {
  content: "";
  position: absolute;
  inset: 35% 0 0;
  background: linear-gradient(to top, rgba(8, 10, 14, 0.85), transparent);
  pointer-events: none;
}

.v2-library-card-media img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.v2-library-card-badges {
  position: absolute;
  z-index: 1;
  inset: var(--v2-space-3) var(--v2-space-3) auto;
  display: flex;
  flex-wrap: wrap;
  gap: var(--v2-space-1);
}

.v2-library-card-badges span {
  padding: 0.32rem 0.55rem;
  border-radius: var(--v2-radius-full);
  background: rgba(8, 10, 14, 0.78);
  color: var(--v2-on-surface-variant);
  font-size: 0.66rem;
  font-weight: 800;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  backdrop-filter: blur(10px);
}

.v2-library-card-badges .v2-library-owned {
  color: var(--v2-secondary);
}

.v2-library-card-badges .v2-library-incompatible {
  color: var(--v2-danger);
}

.v2-library-card-body {
  flex: 1;
  display: grid;
  align-content: start;
  gap: var(--v2-space-3);
  padding: var(--v2-space-4);
}

.v2-library-card-body h2 {
  margin: 0.2rem 0;
  font-size: 1.35rem;
}

.v2-library-scope,
.v2-library-metadata-missing,
.v2-library-no-action {
  margin: 0;
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
  line-height: 1.5;
}

.v2-library-metadata-missing {
  padding: var(--v2-space-2);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-low);
}

.v2-library-card-details {
  display: grid;
  gap: var(--v2-space-2);
  margin: 0;
}

.v2-library-card-details > div {
  display: grid;
  grid-template-columns: minmax(100px, auto) minmax(0, 1fr);
  gap: var(--v2-space-2);
  padding-bottom: var(--v2-space-2);
  border-bottom: 1px solid var(--v2-outline-ghost);
}

.v2-library-card-details dt {
  color: var(--v2-on-surface-variant);
  font-size: 0.74rem;
}

.v2-library-card-details dd {
  min-width: 0;
  margin: 0;
  font-size: 0.76rem;
  overflow-wrap: anywhere;
  text-align: right;
}

.v2-library-card-body > .v2-btn-primary {
  width: 100%;
  margin-top: auto;
}

.v2-library-summary {
  position: sticky;
  top: 96px;
  display: grid;
  gap: var(--v2-space-3);
}

.v2-library-summary-card {
  padding: var(--v2-space-4);
}

.v2-library-summary-card h2 {
  margin: 0.2rem 0 var(--v2-space-3);
}

.v2-library-summary-card dl {
  display: grid;
  gap: var(--v2-space-2);
  margin: 0;
}

.v2-library-summary-card dl > div {
  display: flex;
  justify-content: space-between;
  gap: var(--v2-space-3);
  padding-bottom: var(--v2-space-2);
  border-bottom: 1px solid var(--v2-outline-ghost);
}

.v2-library-summary-card dd {
  margin: 0;
  color: var(--v2-secondary);
  font-weight: 800;
}

.v2-library-summary-card p {
  color: var(--v2-on-surface-variant);
  line-height: 1.55;
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

.v2-achievements,
.v2-community {
  display: grid;
  gap: var(--v2-space-5);
}

.v2-achievements-hero,
.v2-community-hero {
  position: relative;
  overflow: hidden;
  padding: clamp(1.5rem, 4vw, 3rem);
}

.v2-achievements-hero {
  display: flex;
  align-items: center;
  gap: var(--v2-space-5);
  background:
    radial-gradient(circle at 12% 20%, rgba(182, 160, 255, 0.24), transparent 32%),
    linear-gradient(135deg, rgba(27, 32, 40, 0.92), rgba(42, 35, 55, 0.78));
}

.v2-achievements-hero-mark,
.v2-community-unavailable-mark {
  width: 88px;
  height: 88px;
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  border: 1px solid rgba(182, 160, 255, 0.3);
  border-radius: 28px;
  background: rgba(182, 160, 255, 0.12);
  color: var(--v2-primary);
  box-shadow: var(--v2-shadow-glow-primary);
}

.v2-achievements-hero-mark .material-symbols-outlined,
.v2-community-unavailable-mark .material-symbols-outlined {
  font-size: 44px;
}

.v2-achievements-hero h1,
.v2-community-hero h1 {
  margin: var(--v2-space-1) 0 var(--v2-space-2);
  font-size: clamp(2.2rem, 5vw, 4.2rem);
  line-height: 0.95;
}

.v2-achievements-hero > div:last-child > p:last-child,
.v2-community-hero > p:last-child {
  max-width: 68ch;
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.6;
}

.v2-achievement-state {
  min-height: 220px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-4);
  padding: clamp(1.5rem, 5vw, 3rem);
  border: 1px solid var(--v2-outline-ghost);
  text-align: left;
}

.v2-achievement-state > .material-symbols-outlined {
  color: var(--v2-secondary);
  font-size: 42px;
}

.v2-achievement-state-error > .material-symbols-outlined {
  color: var(--v2-danger);
}

.v2-achievement-state h2,
.v2-achievement-state p {
  margin: 0;
}

.v2-achievement-state p {
  margin-top: var(--v2-space-1);
  color: var(--v2-on-surface-variant);
}

.v2-achievement-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
  gap: var(--v2-space-4);
}

.v2-achievement-card {
  min-width: 0;
  display: grid;
  grid-template-rows: auto 1fr auto;
  gap: var(--v2-space-4);
  padding: var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: linear-gradient(155deg, var(--v2-surface-high), var(--v2-surface-low));
  box-shadow: var(--v2-shadow-ambient);
}

.v2-achievement-art {
  min-height: 170px;
  display: grid;
  place-items: center;
  overflow: hidden;
  border: 1px solid rgba(182, 160, 255, 0.18);
  border-radius: var(--v2-radius-lg);
  background:
    radial-gradient(circle at center, rgba(182, 160, 255, 0.2), transparent 58%),
    var(--v2-surface-lowest);
}

.v2-achievement-art img {
  width: 112px;
  height: 112px;
  border-radius: var(--v2-radius-lg);
  object-fit: cover;
  box-shadow: var(--v2-shadow-glow-primary);
}

.v2-achievement-art .material-symbols-outlined {
  color: var(--v2-primary);
  font-size: 68px;
}

.v2-achievement-copy h2 {
  margin: var(--v2-space-1) 0 var(--v2-space-2);
  font-family: var(--v2-font-display);
  font-size: 1.25rem;
}

.v2-achievement-copy > p:last-child {
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.55;
}

.v2-achievement-meta {
  display: grid;
  gap: var(--v2-space-2);
  margin: 0;
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
}

.v2-achievement-meta > div {
  min-width: 0;
  display: flex;
  justify-content: space-between;
  gap: var(--v2-space-3);
}

.v2-achievement-meta dt,
.v2-achievement-meta dd {
  margin: 0;
  font-size: 0.78rem;
}

.v2-achievement-meta dt {
  color: var(--v2-on-surface-variant);
}

.v2-achievement-meta dd {
  overflow-wrap: anywhere;
  color: var(--v2-secondary);
  font-weight: 700;
  text-align: right;
}

.v2-community-hero {
  min-height: 250px;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  background:
    radial-gradient(circle at 82% 18%, rgba(0, 210, 253, 0.18), transparent 34%),
    radial-gradient(circle at 16% 90%, rgba(255, 150, 187, 0.14), transparent 35%),
    rgba(27, 32, 40, 0.72);
}

.v2-community-unavailable {
  min-height: 300px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-5);
  padding: clamp(1.5rem, 6vw, 4rem);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-community-unavailable h2 {
  margin: var(--v2-space-1) 0 var(--v2-space-2);
  font-family: var(--v2-font-display);
  font-size: clamp(1.5rem, 3vw, 2.4rem);
}

.v2-community-unavailable > div:last-child > p:last-child {
  max-width: 62ch;
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.65;
}

.v2-badge-showcase {
  min-width: 0;
  padding: var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-badge-showcase-header {
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
  margin-bottom: var(--v2-space-4);
}

.v2-badge-showcase-header > .material-symbols-outlined {
  width: 44px;
  height: 44px;
  display: grid;
  place-items: center;
  border-radius: var(--v2-radius-md);
  background: rgba(182, 160, 255, 0.12);
  color: var(--v2-primary);
}

.v2-badge-showcase-header h3,
.v2-badge-showcase-state {
  margin: 0;
}

.v2-badge-showcase-state {
  color: var(--v2-on-surface-variant);
  line-height: 1.55;
}

.v2-badge-showcase-error {
  color: var(--v2-danger);
}

.v2-badge-showcase-row {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-badge-chip {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
  padding: var(--v2-space-2);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-low);
}

.v2-badge-chip-art {
  width: 52px;
  height: 52px;
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  overflow: hidden;
  border-radius: var(--v2-radius-md);
  background: rgba(182, 160, 255, 0.12);
  color: var(--v2-primary);
}

.v2-badge-chip-art img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.v2-badge-chip > div:last-child {
  min-width: 0;
  display: grid;
  gap: 0.2rem;
}

.v2-badge-chip strong,
.v2-badge-chip span {
  overflow-wrap: anywhere;
}

.v2-badge-chip span {
  color: var(--v2-on-surface-variant);
  font-size: 0.75rem;
}

.v2-badge-chip .v2-badge-chip-visibility {
  color: var(--v2-secondary);
}

.v2-purchases {
  display: grid;
  gap: var(--v2-space-5);
}

.v2-purchases-hero {
  min-height: 240px;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  padding: clamp(1.5rem, 5vw, 3rem);
  background:
    radial-gradient(circle at 82% 20%, rgba(0, 210, 253, 0.18), transparent 32%),
    linear-gradient(145deg, rgba(27, 32, 40, 0.9), rgba(29, 43, 52, 0.75));
}

.v2-purchases-hero h1 {
  margin: var(--v2-space-1) 0 var(--v2-space-2);
  font-size: clamp(2.2rem, 5vw, 4rem);
  line-height: 0.98;
}

.v2-purchases-hero > p:last-child {
  max-width: 64ch;
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.6;
}

.v2-purchase-state {
  min-height: 230px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-4);
  padding: clamp(1.5rem, 5vw, 3rem);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-purchase-state > .material-symbols-outlined {
  color: var(--v2-secondary);
  font-size: 44px;
}

.v2-purchase-state-error > .material-symbols-outlined {
  color: var(--v2-danger);
}

.v2-purchase-state h2,
.v2-purchase-state p {
  margin: 0;
}

.v2-purchase-state p {
  margin-top: var(--v2-space-1);
  color: var(--v2-on-surface-variant);
}

.v2-purchases-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--v2-space-4);
  padding: var(--v2-space-3) var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-purchases-toolbar .v2-tab-row {
  min-width: 0;
  flex-wrap: wrap;
  margin: 0;
}

.v2-purchases-toolbar .v2-tab:focus-visible {
  outline: 2px solid var(--v2-secondary);
  outline-offset: 2px;
}

.v2-purchases-partial {
  margin: 0;
  color: var(--v2-tertiary);
  font-size: 0.8rem;
}

.v2-purchase-list {
  display: grid;
  gap: var(--v2-space-3);
}

.v2-purchase-record {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  gap: var(--v2-space-4);
  align-items: center;
  padding: var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
}

.v2-purchase-record-mark {
  width: 56px;
  height: 56px;
  display: grid;
  place-items: center;
  border-radius: var(--v2-radius-lg);
  background: rgba(0, 210, 253, 0.1);
  color: var(--v2-secondary);
}

.v2-purchase-record-copy {
  min-width: 0;
}

.v2-purchase-record-copy h2 {
  margin: var(--v2-space-1) 0;
  font-family: var(--v2-font-display);
  font-size: 1.2rem;
}

.v2-purchase-coordinate {
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
}

.v2-purchase-validation {
  margin: var(--v2-space-2) 0 0;
  color: var(--v2-danger);
  font-size: 0.82rem;
}

.v2-purchase-record-summary {
  display: grid;
  justify-items: end;
  gap: var(--v2-space-1);
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
  text-align: right;
}

.v2-purchase-status {
  padding: 0.3rem 0.65rem;
  border-radius: var(--v2-radius-full);
  font-weight: 800;
}

.v2-purchase-status-active {
  background: rgba(0, 210, 253, 0.1);
  color: var(--v2-secondary);
}

.v2-purchase-status-inactive {
  background: rgba(255, 150, 187, 0.1);
  color: var(--v2-tertiary);
}

.v2-purchase-status-error {
  background: rgba(255, 96, 96, 0.1);
  color: var(--v2-danger);
}

.v2-purchase-technical {
  grid-column: 2 / -1;
  min-width: 0;
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
}

.v2-purchase-technical summary {
  width: fit-content;
  cursor: pointer;
}

.v2-purchase-technical summary:focus-visible {
  outline: 2px solid var(--v2-secondary);
  outline-offset: 3px;
}

.v2-purchase-technical dl {
  display: grid;
  gap: var(--v2-space-2);
  margin: var(--v2-space-3) 0 0;
}

.v2-purchase-technical dl > div {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--v2-space-3);
}

.v2-purchase-technical dt,
.v2-purchase-technical dd {
  margin: 0;
}

.v2-purchase-technical dd {
  overflow-wrap: anywhere;
  color: var(--v2-on-background);
  text-align: right;
}

.badge-earned-modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  overflow-y: auto;
  padding: var(--v2-space-4);
  background: rgba(8, 10, 15, 0.78);
  backdrop-filter: blur(16px);
}

.badge-earned-modal-panel {
  position: relative;
  width: min(100%, 520px);
  max-height: calc(100vh - 2rem);
  overflow-y: auto;
  padding: clamp(1.5rem, 5vw, 2.5rem);
  border: 1px solid rgba(182, 160, 255, 0.26);
  border-radius: var(--v2-radius-xl);
  background:
    radial-gradient(circle at 50% 12%, rgba(182, 160, 255, 0.2), transparent 34%),
    var(--v2-surface-low);
  box-shadow: var(--v2-shadow-ambient), var(--v2-shadow-glow-primary);
}

.badge-earned-modal-close {
  position: absolute;
  top: var(--v2-space-3);
  right: var(--v2-space-3);
  width: 44px;
  height: 44px;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-full);
  background: var(--v2-surface-highest);
  color: var(--v2-on-background);
  cursor: pointer;
  font-size: 1.5rem;
}

.badge-earned-modal-close:focus-visible {
  outline: 2px solid var(--v2-secondary);
  outline-offset: 2px;
}

.badge-earned-modal-content {
  display: grid;
  justify-items: center;
  text-align: center;
}

.badge-earned-modal-eyebrow {
  margin: 0 3rem var(--v2-space-1);
  color: var(--v2-tertiary);
  font-size: 0.75rem;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.badge-earned-modal-content h3 {
  margin: 0 3rem var(--v2-space-4);
  font-family: var(--v2-font-display);
  font-size: clamp(1.6rem, 5vw, 2.4rem);
}

.badge-earned-modal-art {
  width: 160px;
  height: 160px;
  display: grid;
  place-items: center;
  margin-bottom: var(--v2-space-4);
  overflow: hidden;
  border: 1px solid rgba(182, 160, 255, 0.24);
  border-radius: 34px;
  background: rgba(182, 160, 255, 0.12);
  color: var(--v2-primary);
}

.badge-earned-modal-art > .material-symbols-outlined {
  font-size: 80px;
}

.badge-earned-modal-image {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.badge-earned-modal-content h4 {
  margin: 0 0 var(--v2-space-2);
  font-family: var(--v2-font-display);
  font-size: 1.5rem;
}

.badge-earned-modal-content > p:not(.badge-earned-modal-eyebrow) {
  margin: 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.6;
}

.badge-earned-modal-meta {
  width: 100%;
  display: grid;
  gap: var(--v2-space-2);
  margin: var(--v2-space-4) 0 0;
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
  text-align: left;
}

.badge-earned-modal-meta > div {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--v2-space-3);
}

.badge-earned-modal-meta dt,
.badge-earned-modal-meta dd {
  margin: 0;
  font-size: 0.78rem;
}

.badge-earned-modal-meta dt {
  color: var(--v2-on-surface-variant);
}

.badge-earned-modal-meta dd {
  overflow-wrap: anywhere;
  color: var(--v2-secondary);
  text-align: right;
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

.v2-auth-screen {
  position: relative;
  overflow: hidden;
  background:
    radial-gradient(circle at 12% 12%, rgba(186, 132, 255, 0.14), transparent 32rem),
    radial-gradient(circle at 88% 76%, rgba(48, 199, 224, 0.09), transparent 28rem),
    var(--v2-background);
}

.v2-auth-topbar {
  min-height: 72px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--v2-space-3) clamp(1rem, 4vw, 3rem);
  border-bottom: 1px solid var(--v2-outline-ghost);
  background: rgba(14, 17, 22, 0.62);
  backdrop-filter: blur(18px);
}

.v2-auth-brand {
  display: flex;
  align-items: center;
  gap: var(--v2-space-2);
  font-family: var(--v2-font-display);
  font-weight: 800;
}

.v2-auth-brand-mark {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  background: linear-gradient(135deg, var(--v2-primary), var(--v2-secondary));
  clip-path: polygon(0 0, 100% 0, 100% 100%, 38% 100%, 38% 62%, 0 62%);
}

.v2-auth-main {
  width: min(1180px, calc(100% - 2rem));
  min-height: calc(100vh - 72px);
  margin: 0 auto;
  display: grid;
  align-items: center;
  padding: clamp(2rem, 6vw, 5rem) 0;
}

.v2-auth-heading {
  max-width: 760px;
  margin-bottom: var(--v2-space-5);
}

.v2-auth-heading h1 {
  margin: 0.15em 0;
  font-size: clamp(2.6rem, 7vw, 5.8rem);
  line-height: 0.94;
}

.v2-auth-heading > p:last-child,
.v2-auth-method-card > p,
.v2-auth-focus-card > p,
.v2-settings-card > p {
  color: var(--v2-on-surface-variant);
  line-height: 1.55;
}

.v2-auth-account-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-auth-account-card,
.v2-auth-add-card,
.v2-auth-method-card,
.v2-auth-focus-card {
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: rgba(29, 33, 42, 0.72);
  box-shadow: var(--v2-shadow-ambient);
}

.v2-auth-account-card {
  min-width: 0;
  display: grid;
  gap: var(--v2-space-3);
  padding: var(--v2-space-4);
}

.v2-auth-account-card-active {
  border-color: rgba(186, 132, 255, 0.5);
  box-shadow: var(--v2-shadow-ambient), 0 0 28px rgba(186, 132, 255, 0.13);
}

.v2-auth-account-identity,
.v2-settings-active-account {
  display: flex;
  align-items: center;
  gap: var(--v2-space-3);
}

.v2-auth-account-avatar,
.v2-settings-avatar {
  width: 68px;
  height: 68px;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
  box-shadow: 0 0 0 3px rgba(186, 132, 255, 0.2);
}

.v2-auth-account-fallback,
.v2-settings-avatar-fallback {
  display: grid;
  place-items: center;
  background: linear-gradient(145deg, var(--v2-primary-container), var(--v2-surface-bright));
  font-family: var(--v2-font-display);
  font-size: 1.45rem;
  font-weight: 800;
}

.v2-auth-account-copy {
  min-width: 0;
}

.v2-auth-account-copy h2,
.v2-settings-active-account h3 {
  margin: 0;
}

.v2-auth-account-copy p,
.v2-settings-active-account p {
  margin: 0.25rem 0 0;
  color: var(--v2-on-surface-variant);
  overflow-wrap: anywhere;
}

.v2-auth-account-statuses {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--v2-space-2);
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
}

.v2-auth-account-actions,
.v2-settings-actions,
.v2-settings-row-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--v2-space-2);
}

.v2-auth-account-actions button,
.v2-settings-actions button,
.v2-settings-row-actions button,
.v2-auth-method-card button,
.v2-auth-focus-card button,
.v2-settings-card > button,
.nip49-modal-actions button,
.nip49-modal-result button {
  min-height: 42px;
  padding: var(--v2-space-2) var(--v2-space-3);
}

.v2-btn-danger {
  border: 1px solid rgba(255, 96, 120, 0.32);
  border-radius: var(--v2-radius-md);
  background: rgba(255, 96, 120, 0.1);
  color: var(--v2-danger);
  font-weight: 700;
  cursor: pointer;
}

.v2-btn-danger:hover:not(:disabled) {
  border-color: var(--v2-danger);
  background: rgba(255, 96, 120, 0.18);
}

.v2-auth-add-card,
.v2-auth-method-link {
  min-height: 230px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-2);
  padding: var(--v2-space-4);
  border-style: dashed;
  color: var(--v2-on-surface-variant);
  text-align: center;
  cursor: pointer;
}

.v2-auth-add-card:hover,
.v2-auth-method-link:hover {
  border-color: var(--v2-primary);
  color: var(--v2-on-background);
}

.v2-auth-method-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--v2-space-4);
}

.v2-auth-method-card,
.v2-auth-focus-card {
  display: grid;
  align-content: start;
  gap: var(--v2-space-3);
  padding: clamp(1.25rem, 4vw, 2.2rem);
}

.v2-auth-method-featured {
  background:
    linear-gradient(145deg, rgba(186, 132, 255, 0.09), transparent 48%),
    rgba(29, 33, 42, 0.78);
}

.v2-auth-method-icon {
  width: 48px;
  height: 48px;
  display: grid;
  place-items: center;
  border-radius: var(--v2-radius-md);
  background: var(--v2-primary-container);
  color: var(--v2-primary);
}

.v2-auth-method-card h2,
.v2-auth-focus-card h1 {
  margin: 0;
}

.v2-auth-method-card label,
.v2-auth-focus-card label,
.nip49-modal-field label,
.nip49-modal-result label {
  font-size: 0.8rem;
  font-weight: 700;
}

.v2-auth-focus-card {
  width: min(680px, 100%);
  margin: 0 auto;
}

.v2-auth-qr {
  width: min(300px, 100%);
  margin: var(--v2-space-3) auto;
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-lg);
  background: white;
}

.v2-auth-qr svg {
  width: 100%;
  height: auto;
}

.v2-auth-live,
.v2-auth-success,
.v2-settings-alert {
  color: var(--v2-secondary);
}

.v2-auth-error,
.v2-settings-alert-error {
  color: var(--v2-error);
}

.v2-auth-global-status,
.v2-auth-toast {
  position: fixed;
  right: var(--v2-space-4);
  bottom: var(--v2-space-4);
  z-index: 110;
  max-width: min(420px, calc(100% - 2rem));
  padding: var(--v2-space-3) var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-highest);
  box-shadow: var(--v2-shadow-ambient);
}

.v2-auth-toast {
  color: var(--v2-error);
}

.v2-confirm-backdrop,
.nip49-modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 120;
  display: grid;
  place-items: center;
  padding: var(--v2-space-4);
  background: rgba(5, 7, 10, 0.8);
  backdrop-filter: blur(12px);
}

dialog.v2-confirm-backdrop {
  width: 100%;
  max-width: none;
  height: 100%;
  max-height: none;
  margin: 0;
  border: 0;
  color: var(--v2-on-background);
}

dialog.v2-confirm-backdrop:not([open]) {
  display: none;
}

dialog.v2-confirm-backdrop::backdrop {
  background: rgba(5, 7, 10, 0.8);
  backdrop-filter: blur(12px);
}

.nip49-modal-backdrop {
  width: 100%;
  max-width: none;
  height: 100%;
  max-height: none;
  margin: 0;
  border: 0;
  color: var(--v2-on-background);
}

.nip49-modal-backdrop:not([open]) {
  display: none;
}

.nip49-modal-backdrop::backdrop {
  background: rgba(5, 7, 10, 0.8);
  backdrop-filter: blur(12px);
}

.v2-confirm-dialog,
.nip49-modal-panel {
  width: min(600px, 100%);
  max-height: min(90vh, 760px);
  overflow: auto;
  padding: var(--v2-space-5);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: var(--v2-surface-high);
  box-shadow: var(--v2-shadow-ambient);
}

.v2-confirm-dialog h2 {
  margin-top: 0;
}

.nip49-modal-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--v2-space-3);
}

.nip49-modal-header h2 {
  margin: 0;
}

.nip49-modal-close {
  width: 40px;
  height: 40px;
  border: 0;
  border-radius: 50%;
  background: var(--v2-surface-highest);
  color: var(--v2-on-background);
}

.nip49-modal-warning,
.nip49-modal-npub,
.nip49-modal-result-message {
  color: var(--v2-on-surface-variant);
  overflow-wrap: anywhere;
}

.nip49-modal-field,
.nip49-modal-result,
.nip49-modal-copy-confirm {
  display: grid;
  gap: var(--v2-space-2);
  margin-top: var(--v2-space-3);
}

.nip49-modal-field input,
.nip49-modal-result-text {
  width: 100%;
  padding: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-lowest);
  color: var(--v2-on-background);
}

.nip49-modal-result-text {
  overflow-wrap: anywhere;
}

.nip49-modal-error {
  color: var(--v2-error);
}

.nip49-modal-copy-status {
  color: var(--v2-secondary);
}

.nip49-modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--v2-space-2);
  margin-top: var(--v2-space-4);
}

.nip49-modal-cancel,
.nip49-modal-copy {
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: transparent;
  color: var(--v2-on-background);
}

.nip49-modal-export {
  border: 0;
  border-radius: var(--v2-radius-md);
  background: linear-gradient(120deg, var(--v2-primary), var(--v2-primary-dim));
  color: var(--v2-on-primary);
  font-weight: 700;
}

.v2-settings-wrap,
.v2-profile-wrap {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-settings-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--v2-space-4);
}

.v2-settings-card,
.v2-profile-card {
  min-width: 0;
  display: grid;
  align-content: start;
  gap: var(--v2-space-3);
  padding: var(--v2-space-4);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: var(--v2-surface-high);
}

.v2-settings-account-card,
.v2-settings-diagnostics {
  grid-column: 1 / -1;
}

.v2-settings-card-header,
.v2-profile-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--v2-space-3);
}

.v2-settings-card-header > div {
  flex: 1;
}

.v2-settings-card-header h2,
.v2-profile-section-header h2 {
  margin: 0;
}

.v2-settings-icon {
  width: 44px;
  height: 44px;
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  border-radius: var(--v2-radius-md);
  background: var(--v2-primary-container);
  color: var(--v2-primary);
}

.v2-settings-active-account {
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-low);
}

.v2-settings-account-list,
.v2-settings-relay-list {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-settings-account-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--v2-space-3);
  padding: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
}

.v2-settings-account-row > div:first-child {
  min-width: 0;
  display: grid;
  gap: 0.2rem;
}

.v2-settings-account-row span,
.v2-settings-muted {
  color: var(--v2-on-surface-variant);
  font-size: 0.82rem;
  overflow-wrap: anywhere;
}

.v2-settings-relay-list {
  max-height: 180px;
  margin: 0;
  padding: var(--v2-space-3);
  overflow: auto;
  list-style: none;
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-lowest);
  color: var(--v2-on-surface-variant);
  font-size: 0.8rem;
}

.v2-settings-toggle-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--v2-space-3);
  padding: var(--v2-space-3);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
}

.v2-settings-toggle-row span {
  display: grid;
  gap: 0.3rem;
}

.v2-settings-toggle-row small {
  color: var(--v2-on-surface-variant);
}

.v2-settings-toggle-row input {
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  accent-color: var(--v2-secondary);
}

.v2-settings-diagnostic-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--v2-space-2);
}

.v2-settings-diagnostic-grid div {
  min-width: 0;
  display: grid;
  gap: 0.3rem;
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-low);
}

.v2-settings-diagnostic-grid span {
  color: var(--v2-on-surface-variant);
  font-size: 0.76rem;
}

.v2-settings-diagnostic-summary {
  max-height: 180px;
  overflow: auto;
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-lowest);
  color: var(--v2-on-surface-variant);
  white-space: pre-wrap;
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

.v2-profile-hero {
  min-height: 300px;
  display: flex;
  align-items: flex-end;
  gap: var(--v2-space-5);
  padding: clamp(1.5rem, 5vw, 3rem);
  background:
    radial-gradient(circle at 75% 15%, rgba(48, 199, 224, 0.12), transparent 38%),
    radial-gradient(circle at 15% 75%, rgba(186, 132, 255, 0.17), transparent 40%),
    var(--v2-surface-high);
}

.v2-profile-avatar {
  width: clamp(110px, 16vw, 170px);
  height: clamp(110px, 16vw, 170px);
  border-radius: 28%;
  object-fit: cover;
  box-shadow: 0 0 0 4px rgba(255, 255, 255, 0.06), var(--v2-shadow-ambient);
}

.v2-profile-avatar-fallback {
  display: grid;
  place-items: center;
  background: linear-gradient(145deg, var(--v2-primary-container), var(--v2-surface-bright));
  font-family: var(--v2-font-display);
  font-size: 3rem;
  font-weight: 900;
}

.v2-profile-identity {
  min-width: 0;
}

.v2-profile-identity h1 {
  margin: 0.1em 0;
  font-size: clamp(2.5rem, 6vw, 5rem);
  line-height: 0.95;
}

.v2-profile-username,
.v2-profile-npub,
.v2-profile-muted,
.v2-profile-readonly-note {
  color: var(--v2-on-surface-variant);
  overflow-wrap: anywhere;
}

.v2-profile-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 340px;
  gap: var(--v2-space-4);
  align-items: start;
}

.v2-profile-main {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-profile-badges {
  position: sticky;
  top: 96px;
  min-width: 0;
}

.v2-profile-about {
  color: var(--v2-on-surface-variant);
  line-height: 1.7;
  white-space: pre-wrap;
}

.v2-profile-metadata {
  display: grid;
  gap: var(--v2-space-2);
}

.v2-profile-metadata > div {
  display: flex;
  justify-content: space-between;
  gap: var(--v2-space-3);
  padding: var(--v2-space-3);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-low);
}

.v2-profile-metadata dt {
  color: var(--v2-on-surface-variant);
}

.v2-profile-metadata dd {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--v2-space-2);
  margin: 0;
  overflow-wrap: anywhere;
  text-align: right;
}

.v2-profile-metadata a {
  color: var(--v2-secondary);
}

.v2-profile-readonly-note {
  padding-top: var(--v2-space-3);
  border-top: 1px solid var(--v2-outline-ghost);
  font-size: 0.8rem;
}

.v2-profile-listings-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--v2-space-3);
}

.v2-profile-listing-card {
  min-width: 0;
  display: grid;
  grid-template-columns: 100px minmax(0, 1fr);
  align-items: center;
  gap: var(--v2-space-3);
  padding: var(--v2-space-2);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-low);
  color: var(--v2-on-background);
  text-align: left;
}

.v2-profile-listing-card img {
  width: 100px;
  aspect-ratio: 16 / 10;
  border-radius: var(--v2-radius-md);
  object-fit: cover;
}

.v2-profile-listing-card span {
  min-width: 0;
  display: grid;
  gap: 0.35rem;
}

.v2-profile-listing-card small {
  color: var(--v2-on-surface-variant);
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

.v2-btn-primary,
.v2-btn-secondary,
.v2-btn-ghost,
.v2-btn-danger {
  min-height: 42px;
  padding: 0.7rem 1rem;
  border-radius: var(--v2-radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--v2-space-2);
  font: inherit;
  font-weight: 700;
  line-height: 1.2;
  text-align: center;
  cursor: pointer;
  transition: background-color 150ms ease, border-color 150ms ease, box-shadow 150ms ease,
    color 150ms ease, opacity 150ms ease, transform 150ms ease;
}

.v2-btn-primary,
.v2-btn-secondary {
  border: none;
}

.v2-btn-primary {
  background: linear-gradient(120deg, var(--v2-primary) 0%, var(--v2-primary-dim) 100%);
  color: var(--v2-on-primary);
}

.v2-btn-primary:hover:not(:disabled) {
  box-shadow: var(--v2-shadow-glow-primary);
}

.v2-btn-secondary {
  background: var(--v2-secondary);
  color: var(--v2-on-secondary);
}

.v2-btn-secondary:hover:not(:disabled) {
  background: var(--v2-secondary-dim);
}

.v2-btn-ghost {
  border: 1px solid transparent;
  background: transparent;
  color: var(--v2-on-background);
}

.v2-btn-ghost:hover:not(:disabled) {
  background: rgba(32, 38, 47, 0.3);
}

.v2-btn-primary:focus-visible,
.v2-btn-secondary:focus-visible,
.v2-btn-ghost:focus-visible,
.v2-btn-danger:focus-visible {
  outline: 2px solid var(--v2-primary);
  outline-offset: 2px;
}

.v2-btn-primary:active:not(:disabled),
.v2-btn-secondary:active:not(:disabled),
.v2-btn-ghost:active:not(:disabled),
.v2-btn-danger:active:not(:disabled) {
  transform: translateY(1px);
}

.v2-btn-primary:disabled,
.v2-btn-secondary:disabled,
.v2-btn-ghost:disabled,
.v2-btn-danger:disabled {
  opacity: 0.55;
  cursor: not-allowed;
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

/* Publisher studio: intentionally scoped so management density does not leak elsewhere. */
.v2-publisher-studio {
  display: grid;
  gap: 1.5rem;
  max-width: 1480px;
  margin: 0 auto;
}

.v2-publisher-studio h1,
.v2-publisher-studio h2,
.v2-publisher-studio h3,
.v2-publisher-studio p {
  margin-top: 0;
}

.v2-publisher-studio h1 {
  margin-bottom: 0.65rem;
  font-family: var(--v2-font-display);
  font-size: clamp(2.25rem, 5vw, 4.8rem);
  line-height: 0.96;
  letter-spacing: -0.045em;
}

.v2-publisher-studio h2 {
  margin-bottom: 1rem;
  font-family: var(--v2-font-display);
  font-size: clamp(1.3rem, 2vw, 1.8rem);
}

.v2-publisher-kicker {
  margin-bottom: 0.55rem;
  color: var(--v2-primary);
  font-size: 0.72rem;
  font-weight: 800;
  letter-spacing: 0.22em;
  text-transform: uppercase;
}

.v2-publisher-header,
.v2-publisher-game-hero {
  position: relative;
  overflow: hidden;
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 1.5rem;
  padding: clamp(1.35rem, 3vw, 2.6rem);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background:
    radial-gradient(circle at 90% 0%, oklch(0.78 0.16 300 / 20%), transparent 38%),
    linear-gradient(145deg, var(--v2-surface-low), var(--v2-surface-high));
  box-shadow: var(--v2-shadow-ambient);
}

.v2-publisher-game-hero {
  align-items: center;
}

.v2-publisher-game-hero img,
.v2-publisher-game-hero > div:first-child:not(:last-child) {
  flex: 0 0 auto;
}

.v2-publisher-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.7rem;
}

.v2-publisher-actions-end {
  justify-content: flex-end;
  padding-top: 0.5rem;
}

.v2-publisher-studio .v2-btn-primary,
.v2-publisher-studio .v2-btn-secondary {
  min-height: 42px;
  padding: 0.7rem 1rem;
}

.v2-publisher-back {
  width: fit-content;
}

.v2-publisher-game-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
}

.v2-publisher-game-card,
.v2-publisher-panel,
.v2-publisher-promotion-row {
  min-width: 0;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: linear-gradient(145deg, var(--v2-surface-high), var(--v2-surface-low));
}

.v2-publisher-game-card {
  display: flex;
  gap: 1.15rem;
  padding: 1.15rem;
  overflow: hidden;
}

.v2-publisher-management-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(250px, 0.32fr);
  align-items: start;
  gap: 1.25rem;
}

.v2-publisher-main,
.v2-publisher-promotion-list,
.v2-publisher-form {
  display: grid;
  gap: 1rem;
}

.v2-publisher-panel {
  padding: clamp(1.15rem, 2.5vw, 1.8rem);
}

.v2-publisher-sidebar {
  position: sticky;
  top: 92px;
  display: grid;
  gap: 1rem;
}

.v2-publisher-sidebar ul {
  display: grid;
  gap: 0.75rem;
  margin: 0;
  padding-left: 1.1rem;
  color: var(--v2-on-surface-variant);
  line-height: 1.5;
}

.v2-publisher-promotion-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 1rem;
  padding: 1rem;
}

.v2-publisher-promotion-row > p,
.v2-publisher-promotion-row > dialog {
  grid-column: 1 / -1;
}

.v2-publisher-diagnostics {
  margin-top: 0.8rem;
  color: var(--v2-on-surface-variant);
  font-size: 0.78rem;
}

.v2-publisher-diagnostics summary {
  cursor: pointer;
}

.v2-publisher-diagnostics p {
  margin: 0.55rem 0 0;
  overflow-wrap: anywhere;
}

.v2-publisher-form label {
  display: block;
  margin-bottom: 0.45rem;
  font-size: 0.88rem;
  font-weight: 750;
}

.v2-publisher-date-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1rem;
}

.v2-publisher-authority,
.v2-publisher-readonly,
.v2-publisher-option,
.v2-publisher-link-option {
  padding: 1rem;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-highest);
}

.v2-publisher-authority {
  border-left: 3px solid var(--v2-primary);
}

.v2-publisher-authority p,
.v2-publisher-option p,
.v2-publisher-link-option span span {
  margin: 0.35rem 0 0;
  color: var(--v2-on-surface-variant);
  line-height: 1.5;
}

.v2-publisher-readonly {
  border-color: oklch(0.78 0.14 355 / 45%);
}

.v2-publisher-link-option label {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  margin: 0;
}

.v2-publisher-link-option input {
  margin-top: 0.25rem;
  accent-color: var(--v2-primary);
}

.v2-publisher-link-option span span {
  display: block;
}

.v2-publisher-dialog {
  width: min(calc(100% - 2rem), 30rem);
  margin: auto;
  padding: 0;
  border: 0;
  color: var(--v2-on-background);
  background: transparent;
}

.v2-publisher-dialog::backdrop {
  background: rgba(0, 0, 0, 0.76);
  backdrop-filter: blur(5px);
}

.v2-publisher-dialog-card {
  display: grid;
  gap: 1.25rem;
  padding: 1.35rem;
  border: 1px solid var(--v2-outline);
  border-radius: var(--v2-radius-xl);
  background: var(--v2-surface-high);
  box-shadow: var(--v2-shadow-ambient);
}

.v2-publisher-unavailable {
  max-width: 760px;
  padding: clamp(1.5rem, 5vw, 3.5rem);
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-xl);
  background: linear-gradient(145deg, var(--v2-surface-low), var(--v2-surface-high));
}

.v2-publisher-unavailable p:last-child {
  max-width: 62ch;
  color: var(--v2-on-surface-variant);
  line-height: 1.65;
}

/* Typed Store Page editor */
.v2-store-page-editor {
  min-width: 0;
  padding-bottom: 6.5rem;
  overflow-x: clip;
}

.v2-store-editor-tabs {
  position: sticky;
  top: 68px;
  z-index: 30;
  display: flex;
  gap: 0.35rem;
  max-width: 100%;
  padding: 0.55rem;
  overflow-x: auto;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: oklch(0.19 0.02 270 / 94%);
  scrollbar-width: thin;
}

.v2-store-editor-tabs button {
  flex: 0 0 auto;
  min-height: 42px;
  padding: 0.65rem 0.9rem;
  border: 1px solid transparent;
  border-radius: var(--v2-radius-md);
  color: var(--v2-on-surface-variant);
  background: transparent;
  cursor: pointer;
}

.v2-store-editor-tabs button:hover,
.v2-store-editor-tab-active {
  color: var(--v2-on-background) !important;
  border-color: var(--v2-outline-ghost) !important;
  background: var(--v2-surface-highest) !important;
}

.v2-store-editor-tabs button:focus-visible,
.v2-store-page-editor button:focus-visible,
.v2-store-page-editor summary:focus-visible,
.v2-store-page-editor a:focus-visible {
  outline: 2px solid var(--v2-secondary);
  outline-offset: 2px;
}

.v2-store-field-label {
  display: block;
  margin-bottom: 0.45rem;
  font-weight: 750;
}

.v2-store-chip-row,
.v2-store-add-row,
.v2-store-section-heading,
.v2-store-card-actions,
.v2-store-preview-banner,
.v2-store-language-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.65rem;
}

.v2-store-section-heading {
  justify-content: space-between;
  margin-bottom: 1rem;
}

.v2-store-section-heading h2 {
  margin-bottom: 0;
}

.v2-store-add-row .v2-input {
  min-width: min(18rem, 100%);
}

.v2-store-card {
  min-width: 0;
  margin-top: 1rem;
  padding: 1rem;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-high);
}

.v2-store-card .v2-input,
.v2-store-tier .v2-input {
  border-color: color-mix(in oklch, var(--v2-outline) 58%, transparent);
  background-color: var(--v2-surface-lowest) !important;
}

.v2-store-card-actions {
  justify-content: flex-end;
  margin-bottom: 0.8rem;
}

.v2-store-card-actions button,
.v2-store-toolbar button,
.v2-store-preview-banner button,
.v2-store-language-row button,
.v2-store-overflow button {
  min-height: 36px;
  padding: 0.45rem 0.7rem;
  border: 1px solid var(--v2-outline);
  border-radius: var(--v2-radius-sm);
  color: var(--v2-on-background);
  background: var(--v2-surface-high);
  cursor: pointer;
}

.v2-store-form-grid,
.v2-store-tier {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.8rem;
}

.v2-store-tier {
  margin-top: 1rem;
  padding: 1rem;
  border: 1px solid var(--v2-outline-ghost);
  border-radius: var(--v2-radius-md);
  background: var(--v2-surface-highest);
}

.v2-store-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
  margin-bottom: 0.7rem;
}

/* Markdown destinations belong to the typed Links fields, never the prose toolbar. */
.v2-store-toolbar button:nth-child(3) {
  display: none;
}

.v2-store-diagnostic-list button {
  padding: 0;
  border: 0;
  color: var(--v2-secondary);
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.v2-store-canonical-placeholder {
  padding: 2rem;
  border: 1px dashed var(--v2-outline);
  border-radius: var(--v2-radius-lg);
  text-align: center;
}

.v2-store-media-preview {
  display: block;
  width: min(100%, 34rem);
  max-height: 20rem;
  margin-top: 1rem;
  border-radius: var(--v2-radius-md);
  object-fit: contain;
  background: #000;
}

#store-editor-media .v2-store-card::after {
  content: 'Draft media is not loaded here. Validate to use the canonical preview.';
  display: block;
  margin-top: 1rem;
  padding: 0.8rem;
  border: 1px dashed var(--v2-outline);
  border-radius: var(--v2-radius-md);
  color: var(--v2-on-surface-variant);
  font-size: 0.82rem;
}

.v2-store-mono {
  overflow-wrap: anywhere;
  font-family: ui-monospace, monospace;
  font-size: 0.75rem;
}

.v2-store-accessibility-row {
  display: grid;
  grid-template-columns: minmax(10rem, 0.6fr) minmax(8rem, 0.35fr) minmax(14rem, 1fr);
  align-items: center;
  gap: 1rem;
  padding: 0.8rem 0;
  border-bottom: 1px solid var(--v2-outline-ghost);
}

.v2-store-accessibility-row small,
.v2-store-accessibility-row strong {
  display: block;
}

.v2-store-preview {
  width: 100%;
  margin-inline: auto;
  padding: 1rem;
  border: 2px solid var(--v2-primary);
  border-radius: var(--v2-radius-xl);
  transition: max-width 180ms ease;
}

.v2-store-preview-narrow {
  max-width: 430px;
}

.v2-store-preview-banner {
  margin: -1rem -1rem 1rem;
  padding: 0.85rem 1rem;
  background: oklch(0.6 0.24 295 / 20%);
}

.v2-store-readiness-toggle {
  display: none;
  width: fit-content;
}

.v2-store-editor-footer {
  position: fixed;
  right: 1rem;
  bottom: 1rem;
  z-index: 45;
  display: flex;
  align-items: center;
  gap: 0.65rem;
  max-width: calc(100vw - 2rem);
  padding: 0.7rem;
  border: 1px solid var(--v2-outline);
  border-radius: var(--v2-radius-xl);
  background: oklch(0.19 0.02 270 / 96%);
  box-shadow: var(--v2-shadow-ambient);
}

.v2-store-overflow {
  position: relative;
}

.v2-store-overflow > summary {
  list-style: none;
  cursor: pointer;
}

.v2-store-overflow > div {
  position: absolute;
  right: 0;
  bottom: calc(100% + 0.8rem);
  display: grid;
  gap: 0.7rem;
  width: min(22rem, calc(100vw - 2rem));
  padding: 1rem;
  border: 1px solid var(--v2-outline);
  border-radius: var(--v2-radius-lg);
  background: var(--v2-surface-high);
  box-shadow: var(--v2-shadow-ambient);
}

@media (max-width: 1240px) and (min-width: 961px) {
  .v2-detail-hero {
    grid-template-columns: minmax(0, 1fr) 240px;
  }

  .v2-detail-buy-panel {
    grid-column: 1 / -1;
  }
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

  .v2-detail-hero {
    padding: var(--v2-space-4);
  }

  .v2-detail-cover-frame {
    width: min(100%, 360px);
    justify-self: center;
    transform: none;
  }

  .v2-detail-media {
    grid-auto-columns: minmax(250px, 82%);
  }

  .v2-detail-seller-card {
    position: static;
  }

  .v2-detail-campaign-card {
    align-items: flex-start;
    flex-direction: column;
  }

  .v2-auth-account-grid,
  .v2-auth-method-grid,
  .v2-settings-grid,
  .v2-profile-layout,
  .v2-profile-listings-grid,
  .v2-settings-diagnostic-grid,
  .v2-library-layout,
  .v2-library-controls,
  .v2-library-card-grid {
    grid-template-columns: 1fr;
  }

  .v2-publisher-game-grid,
  .v2-publisher-management-layout,
  .v2-publisher-date-grid,
  .v2-publisher-promotion-row {
    grid-template-columns: 1fr;
  }

  .v2-publisher-header,
  .v2-publisher-game-hero {
    align-items: flex-start;
    flex-direction: column;
  }

  .v2-publisher-sidebar {
    position: static;
  }

  .v2-store-editor-tabs {
    top: 80px;
  }

  .v2-store-readiness-toggle {
    display: inline-flex;
  }

  .v2-store-readiness {
    display: none;
  }

  .v2-store-readiness-open {
    display: grid;
  }

  .v2-store-form-grid,
  .v2-store-tier,
  .v2-store-accessibility-row {
    grid-template-columns: 1fr;
  }

  .v2-store-editor-footer {
    right: 0.5rem;
    bottom: calc(76px + 0.5rem);
    display: grid;
    grid-template-columns: minmax(0, 1fr) repeat(3, auto);
    max-width: calc(100vw - 1rem);
  }

  .v2-settings-account-card,
  .v2-settings-diagnostics {
    grid-column: auto;
  }

  .v2-profile-badges {
    position: static;
  }

  .v2-library-summary {
    position: static;
  }

  .v2-profile-hero {
    align-items: flex-start;
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

  .v2-achievements-hero,
  .v2-community-unavailable {
    flex-direction: column;
    align-items: flex-start;
  }

  .v2-purchases-toolbar,
  .v2-purchase-record {
    align-items: flex-start;
    grid-template-columns: 1fr;
  }

  .v2-purchases-toolbar {
    flex-direction: column;
  }

  .v2-purchase-record-summary {
    justify-items: start;
    text-align: left;
  }

  .v2-purchase-technical {
    grid-column: auto;
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

@media (prefers-reduced-motion: reduce) {
  .v2-app *,
  .v2-app *::before,
  .v2-app *::after,
  .v2-auth-screen *,
  .v2-auth-screen *::before,
  .v2-auth-screen *::after {
    animation-delay: 0ms !important;
    animation-duration: 1ms !important;
    animation-iteration-count: 1 !important;
    scroll-behavior: auto !important;
    transition-delay: 0ms !important;
    transition-duration: 1ms !important;
  }

  .v2-nav-item:hover,
  .v2-user-profile-card:hover {
    transform: none;
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_dropdown_options_use_readable_dark_colors() {
        assert!(UI_V2_STYLES.contains("appearance: none;"));
        assert!(UI_V2_STYLES.contains("background-color: var(--v2-surface-highest) !important;"));
        assert!(UI_V2_STYLES.contains("fill='%23a8abb3'"));
        assert!(UI_V2_STYLES.contains(
            "select option {\n  color: var(--v2-on-background);\n  background-color: var(--v2-surface-highest);"
        ));
    }

    #[test]
    fn detail_buy_panel_buttons_are_full_width_touch_targets() {
        let buy_panel_rule = ".v2-detail-buy-panel .v2-btn-primary";
        let rule_start = UI_V2_STYLES
            .find(buy_panel_rule)
            .expect("detail buy panel button rule should exist");
        let rule = &UI_V2_STYLES[rule_start..];
        let rule = &rule[..rule.find('}').expect("rule should close")];

        assert!(rule.contains("width: 100%;"));
        assert!(rule.contains("max-width: 100%;"));
        assert!(rule.contains("min-width: 0;"));
        assert!(rule.contains("min-height:"));
        assert!(rule.contains("padding:"));
        assert!(rule.contains("overflow-wrap: anywhere;"));
    }

    #[test]
    fn detail_buy_panel_metadata_wraps_long_values() {
        let meta_rule = ".v2-detail-buy-panel .v2-social-meta";
        let rule_start = UI_V2_STYLES
            .find(meta_rule)
            .expect("detail buy panel metadata rule should exist");
        let rule = &UI_V2_STYLES[rule_start..];
        let rule = &rule[..rule.find('}').expect("rule should close")];

        assert!(rule.contains("min-width: 0;"));
        assert!(rule.contains("max-width: 100%;"));
        assert!(rule.contains("overflow-wrap: anywhere;"));
    }
}
