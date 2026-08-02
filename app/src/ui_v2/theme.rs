//! Arcadestr Noir design tokens and compatibility classes for UI v2.

pub const UI_V2_STYLES: &str = r#"
:root {
  /* Compatibility aliases: the canonical handoff tokens live in
     web/style/tailwind.css. Keep existing UI v2 consumers on that system. */
  --v2-font-display: var(--arc-font-mono);
  --v2-font-body: var(--arc-font-mono);

  --v2-background: var(--arc-background);
  --v2-on-background: var(--arc-text-primary);
  --v2-surface-lowest: var(--arc-surface-lowest);
  --v2-surface-low: var(--arc-surface);
  --v2-surface: var(--arc-surface);
  --v2-surface-high: var(--arc-surface);
  --v2-surface-highest: var(--arc-surface-recessed);
  --v2-surface-bright: var(--arc-progress-track);

  --v2-primary: var(--arc-accent);
  --v2-primary-dim: oklch(var(--noir-primary-dim));
  --v2-on-primary: var(--arc-background);
  --v2-primary-container: oklch(0.3 0.08 60);
  --v2-on-primary-container: var(--arc-text-heading);

  --v2-secondary: var(--arc-action);
  --v2-secondary-dim: oklch(var(--noir-secondary-dim));
  --v2-on-secondary: var(--arc-background);
  --v2-tertiary: var(--arc-info);
  --v2-on-tertiary: var(--arc-background);

  --v2-outline: var(--arc-border-default);
  --v2-outline-ghost: var(--arc-border-subtle);
  --v2-on-surface-variant: var(--arc-text-muted);
  --v2-danger: var(--arc-error);
  --v2-error: var(--arc-error);
  --v2-success: var(--arc-success);
  --v2-warning: var(--arc-warning);

  --v2-radius-sm: var(--arc-radius-xs);
  --v2-radius-md: var(--arc-radius-sm);
  --v2-radius-lg: var(--arc-radius-md);
  --v2-radius-xl: var(--arc-radius-lg);
  --v2-radius-full: 9999px;

  --v2-space-1: var(--arc-space-1);
  --v2-space-2: var(--arc-space-2);
  --v2-space-3: var(--arc-space-3);
  --v2-space-4: var(--arc-space-4);
  --v2-space-5: var(--arc-space-5);
  --v2-space-6: var(--arc-space-6);
  --v2-space-7: var(--arc-space-7);

  --v2-shadow-ambient: var(--noir-shadow-ambient);
  --v2-shadow-glow-primary: var(--noir-shadow-glow-primary);
  --v2-gradient-primary: var(--noir-gradient-primary);
  --v2-gradient-hero: var(--noir-gradient-hero);
}

* {
  box-sizing: border-box;
}

.arc-app-shell {
  min-height: 100vh;
  background: transparent;
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
}

.arc-topbar {
  position: sticky;
  top: 0;
  z-index: 50;
  height: var(--arc-shell-height);
  border-bottom: 1px solid var(--arc-separator);
  background: var(--arc-surface-lowest);
}

.arc-topbar-inner {
  width: 100%;
  max-width: var(--arc-content-max);
  height: 100%;
  margin: 0 auto;
  padding: 0 var(--arc-page-inline);
  display: flex;
  align-items: center;
  gap: 30px;
}

.arc-logo {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 10px;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--arc-text-heading);
  cursor: pointer;
}

.arc-logo-mark {
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  display: block;
  color: var(--arc-accent);
  fill: currentColor;
}

.arc-logo-wordmark {
  font-family: var(--arc-font-mono);
  font-size: 19px;
  font-weight: 800;
  line-height: 1;
  letter-spacing: var(--arc-tracking-wordmark);
  text-transform: uppercase;
}

.arc-logo:focus-visible,
.arc-primary-nav button:focus-visible,
.arc-search:focus-within,
.arc-relay-control:focus-visible,
.arc-account-control:focus-visible,
.arc-topbar-menu button:focus-visible,
.arc-menu-link:focus-visible,
.arc-menu-close:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.arc-primary-nav {
  display: flex;
  align-items: center;
  gap: 22px;
  white-space: nowrap;
}

.arc-primary-nav button {
  border: 0;
  padding: 4px 0;
  background: transparent;
  color: var(--arc-text-muted);
  font-family: var(--arc-font-mono);
  font-size: 12.5px;
  font-weight: 700;
  line-height: 1;
  text-transform: uppercase;
  cursor: pointer;
  transition: color 120ms ease;
}

.arc-primary-nav button:hover {
  color: var(--arc-text-primary);
}

.arc-primary-nav button.arc-nav-active {
  color: var(--arc-accent);
}

.arc-topbar-actions {
  min-width: 0;
  margin-left: auto;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
}

.arc-search {
  width: 260px;
  min-width: 150px;
  flex: 0 0 260px;
  height: var(--arc-control-sm);
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-sm);
  background: var(--arc-surface-recessed);
}

.arc-search label {
  height: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  color: oklch(0.52 0.01 60);
}

.arc-search .material-symbols-outlined {
  flex: 0 0 auto;
  font-size: 16px;
}

.arc-search input {
  min-width: 0;
  width: 100%;
  height: 100%;
  border: 0;
  margin: 0;
  padding: 0;
  -webkit-appearance: none;
  appearance: none;
  background: transparent;
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
  font-size: 11.5px;
  font-weight: 400;
  line-height: normal;
  outline: none;
}

.arc-search input::placeholder {
  color: oklch(0.52 0.01 60);
  opacity: 1;
}

.arc-topbar-menu-wrap {
  position: relative;
  flex: 0 0 auto;
}

.arc-relay-control,
.arc-account-control {
  height: var(--arc-control-sm);
  display: inline-flex;
  align-items: center;
  background: transparent;
  font-family: var(--arc-font-mono);
  cursor: pointer;
}

.arc-relay-control {
  gap: 7px;
  padding: 6px 10px;
  border: 1px solid oklch(0.45 0.13 195);
  border-radius: var(--arc-radius-xs);
  color: var(--arc-action);
  font-size: 10.5px;
  font-weight: 700;
  white-space: nowrap;
}

.arc-relay-control:hover {
  border-color: var(--arc-action);
  background: oklch(0.8 0.14 195 / 8%);
}

.arc-relay-dot {
  width: 7px;
  height: 7px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--arc-action);
  box-shadow: 0 0 6px var(--arc-action);
}

.arc-relay-dot-offline {
  background: var(--arc-text-subdued);
  box-shadow: none;
}

.arc-account-control {
  max-width: 168px;
  gap: 8px;
  padding: 4px 8px 4px 4px;
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-xs);
  color: var(--arc-text-secondary);
}

.arc-account-control:hover {
  border-color: var(--arc-border-strong);
  color: var(--arc-text-primary);
}

.arc-account-avatar {
  width: 24px;
  height: 24px;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
}

.arc-account-fallback {
  display: grid;
  place-items: center;
  background: var(--arc-progress-track);
  color: var(--arc-text-primary);
  font-size: 10px;
  font-weight: 800;
}

.arc-account-name {
  min-width: 0;
  overflow: hidden;
  color: inherit;
  font-size: 11px;
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-account-chevron {
  flex: 0 0 auto;
  font-size: 15px;
}

.arc-topbar-menu {
  position: absolute;
  top: 38px;
  right: 0;
  z-index: 70;
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface-recessed);
  box-shadow: var(--noir-shadow-ambient);
}

.arc-relay-menu {
  width: min(320px, calc(100vw - 32px));
  max-height: min(360px, calc(100vh - 80px));
  overflow: auto;
  padding: 10px;
}

.arc-account-menu {
  width: 190px;
  display: grid;
  gap: 2px;
  padding: 6px;
}

.arc-account-menu button,
.arc-menu-link {
  width: 100%;
  min-height: 30px;
  border: 0;
  border-radius: var(--arc-radius-xs);
  padding: 7px 9px;
  background: transparent;
  color: var(--arc-text-secondary);
  font-family: var(--arc-font-mono);
  font-size: 11px;
  font-weight: 600;
  text-align: left;
  cursor: pointer;
}

.arc-account-menu button:hover,
.arc-menu-link:hover {
  background: rgb(255 255 255 / 6%);
  color: var(--arc-text-primary);
}

.arc-signer-state {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  margin: 0 3px 4px;
  padding: 4px 6px 8px;
  border-bottom: 1px solid var(--arc-border-subtle);
  color: var(--arc-text-subdued);
  font-size: 10px;
  text-transform: uppercase;
}

.arc-signer-state strong {
  min-width: 0;
  overflow: hidden;
  color: var(--arc-action);
  text-overflow: ellipsis;
}

.arc-account-menu .arc-menu-signout {
  margin-top: 3px;
  border-top: 1px solid var(--arc-border-subtle);
  border-radius: 0 0 var(--arc-radius-xs) var(--arc-radius-xs);
  color: oklch(0.75 0.18 25);
}

.arc-menu-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 8px;
}

.arc-menu-heading h2 {
  margin: 0;
  color: var(--arc-text-primary);
  font-size: 12px;
  font-weight: 800;
  text-transform: uppercase;
}

.arc-menu-close {
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  border: 0;
  background: transparent;
  color: var(--arc-text-muted);
  cursor: pointer;
}

.arc-menu-close .material-symbols-outlined {
  font-size: 17px;
}

.arc-menu-empty {
  margin: 12px 4px;
  color: var(--arc-text-muted);
  font-size: 11px;
  line-height: 1.55;
}

.arc-relay-list {
  display: grid;
  gap: 4px;
  margin: 0 0 8px;
  padding: 0;
  list-style: none;
}

.arc-relay-list li {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-sm);
  background: var(--arc-surface);
  color: var(--arc-text-muted);
  font-size: 10.5px;
  overflow-wrap: anywhere;
}

.arc-page-header {
  min-width: 0;
  margin-bottom: 18px;
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.arc-page-heading {
  min-width: 0;
}

.arc-page-eyebrow {
  margin: 0 0 8px;
  color: var(--arc-accent);
  font-size: 11px;
  font-weight: 800;
  letter-spacing: var(--arc-tracking-label);
  text-transform: uppercase;
}

.arc-page-heading h1 {
  margin: 0;
  color: var(--arc-text-heading);
  font-family: var(--arc-font-mono);
  font-size: 22px;
  font-weight: 800;
  line-height: var(--arc-leading-title);
  letter-spacing: normal;
}

.arc-page-description {
  max-width: 720px;
  margin: 7px 0 0;
  color: var(--arc-text-muted);
  font-size: 12px;
  line-height: 1.6;
}

.arc-page-actions {
  flex: 0 0 auto;
}

.arc-page-container {
  width: 100%;
  max-width: var(--arc-content-standard);
  min-width: 0;
  margin: 0 auto;
  padding: var(--arc-page-block) var(--arc-page-inline) 60px;
}

.arc-page-container-wide {
  max-width: var(--arc-content-max);
}

.arc-page-container-full {
  min-height: calc(100vh - var(--arc-shell-height));
}

@media (max-width: 1120px) {
  .arc-topbar-inner {
    gap: 10px;
  }

  .arc-primary-nav {
    gap: 8px;
  }

  .arc-primary-nav button {
    font-size: 11.5px;
  }

  .arc-search {
    width: 260px;
    min-width: 200px;
  }

  .arc-account-control {
    max-width: 84px;
  }
}

@media (max-width: 820px) {
  .arc-topbar-inner {
    padding-inline: 16px;
  }

  .arc-logo-wordmark {
    display: none;
  }

  .arc-primary-nav {
    gap: 9px;
  }

  .arc-search {
    min-width: 120px;
    width: 18vw;
    flex: 1 1 120px;
  }

  .arc-account-name,
  .arc-account-chevron {
    display: none;
  }

  .arc-account-control {
    width: 32px;
    padding: 4px;
  }

  .arc-page-container {
    padding-inline: 20px;
  }
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
  gap: 14px;
  min-width: 0;
}

.v2-publish-wrap {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-detail-back {
  position: relative;
  width: fit-content;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: auto;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 700;
  text-transform: none;
}

.v2-detail-back::before {
  position: absolute;
  inset: -12px -8px;
  content: "";
}

.v2-detail-description-block {
  padding: 0;
}

.v2-detail-description-block h2 {
  margin: 0 0 var(--v2-space-3) 0;
}

.v2-detail-description-block p {
  margin: 0;
  color: var(--arc-text-secondary);
  line-height: 1.7;
  max-width: 72ch;
}

.v2-detail-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 380px;
  gap: 26px;
  align-items: start;
}

.v2-detail-hero {
  position: relative;
  height: 340px;
  overflow: hidden;
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface-artwork);
}

.v2-detail-hero .arc-game-artwork {
  position: absolute;
  inset: 0;
  min-height: 0;
}

.v2-detail-hero-shade {
  position: absolute;
  inset: 0;
  background: linear-gradient(0deg, rgb(0 0 0 / 58%) 0%, transparent 48%);
}

.v2-detail-title {
  position: absolute;
  left: 24px;
  right: 24px;
  bottom: 20px;
  z-index: 1;
  margin: 0;
  color: #fff;
  font-family: var(--arc-font-mono);
  font-size: clamp(26px, 3.3vw, 34px);
  font-weight: 800;
  line-height: 1.08;
}

.v2-detail-tags {
  margin-top: 12px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.v2-detail-section {
  min-width: 0;
}

.v2-detail-section > .v2-store-kicker {
  margin-bottom: 10px;
}

.v2-detail-wrap .v2-store-kicker {
  color: var(--arc-accent);
}

.v2-detail-about > p:not(.v2-store-kicker) {
  max-width: 76ch;
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 13px;
  line-height: 1.7;
}

.v2-detail-compatibility {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.v2-detail-compatibility-state {
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 600;
}

.v2-detail-release-info {
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 12px;
  line-height: 1.8;
}

.v2-detail-sidebar {
  position: sticky;
  top: calc(var(--arc-shell-height) + 18px);
  display: grid;
  gap: 16px;
  max-height: calc(100vh - var(--arc-shell-height) - 36px);
  overflow-y: auto;
}

.v2-detail-buy-panel {
  padding: 22px;
  display: grid;
  gap: 10px;
  align-content: start;
  min-width: 0;
  max-width: 100%;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
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

.v2-detail-buy-panel > .v2-btn-primary {
  text-transform: uppercase;
}

.v2-detail-buy-panel > * {
  min-width: 0;
}

.v2-detail-buy-panel .v2-social-meta {
  max-width: 100%;
  min-width: 0;
  overflow-wrap: anywhere;
}

.v2-detail-access-states {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 4px;
}

.v2-detail-access-note {
  margin: 0 0 4px;
  color: var(--arc-text-muted);
  font-size: 11.5px;
  line-height: 1.6;
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

.v2-detail-ownership-panel {
  padding: 20px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
}

.v2-detail-ownership-panel p:last-child {
  margin: 10px 0 0;
  color: var(--arc-success);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.55;
}

.v2-install-dialog {
  width: min(440px, calc(100vw - 32px));
  max-height: 86vh;
  margin: auto;
  border: 1px solid var(--arc-border-strong);
  border-radius: 10px;
  padding: 0;
  overflow: auto;
  background: var(--arc-surface);
  color: var(--arc-text-primary);
  box-shadow: 0 22px 70px rgb(0 0 0 / 58%);
}

.v2-install-dialog::backdrop {
  background: rgb(0 0 0 / 76%);
}

.v2-install-dialog-body {
  display: grid;
  gap: 12px;
  padding: 26px;
}

.v2-install-dialog-body h2,
.v2-install-dialog-body p {
  margin: 0;
}

.v2-install-dialog-body > p:not(.v2-store-kicker):not(.v2-install-dialog-note) {
  color: var(--arc-text-secondary);
  font-size: 12px;
  line-height: 1.65;
}

.v2-install-progress {
  position: relative;
  height: 8px;
  overflow: hidden;
  border-radius: 2px;
  background: var(--arc-surface-recessed);
}

.v2-install-progress > span,
.v2-install-progress-indeterminate::after {
  display: block;
  height: 100%;
  background: var(--arc-accent);
}

.v2-install-progress-indeterminate::after {
  width: 36%;
  content: "";
  animation: arc-install-progress 1.15s ease-in-out infinite alternate;
}

@keyframes arc-install-progress {
  from { transform: translateX(-20%); }
  to { transform: translateX(200%); }
}

.v2-install-progress-copy {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  color: var(--arc-text-muted);
  font-size: 11px;
}

.v2-install-progress-copy strong {
  color: var(--arc-text-primary);
}

.v2-install-dialog-note {
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.55;
}

.v2-install-dialog-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  margin-top: 4px;
}

.v2-install-dialog-actions > :only-child {
  grid-column: 1 / -1;
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

.v2-detail-media-stage {
  margin-top: 14px;
}

.v2-detail-rich-media {
  margin: 0;
  overflow: hidden;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface-artwork);
}

.v2-detail-rich-media img,
.v2-detail-rich-media video {
  width: 100%;
  display: block;
  aspect-ratio: 16 / 9;
  object-fit: cover;
}

.v2-detail-rich-media-video {
  background: #000;
}

.v2-detail-rich-media figcaption {
  padding: 10px 12px;
  color: var(--arc-text-muted);
  font-size: 11px;
  line-height: 1.5;
}

.v2-detail-expand-media {
  margin-top: 10px;
}

.v2-detail-media-thumbs {
  display: flex;
  gap: 8px;
  margin-top: 10px;
  padding-bottom: 8px;
  overflow-x: auto;
}

.v2-detail-media-thumb {
  flex: 0 0 auto;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-xs);
  padding: 0;
  overflow: hidden;
  background: transparent;
  cursor: pointer;
}

.v2-detail-media-thumb:hover,
.v2-detail-media-thumb.active {
  border-color: var(--arc-accent);
}

.v2-detail-media-thumb:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.v2-detail-media-thumb img {
  width: 112px;
  height: 64px;
  display: block;
  object-fit: cover;
}

.v2-detail-media-dialog {
  width: min(92vw, 72rem);
  max-height: 92vh;
  margin: auto;
  border: 1px solid var(--arc-border-strong);
  border-radius: var(--arc-radius-md);
  padding: 16px;
  overflow: auto;
  background: var(--arc-surface-raised);
  color: var(--arc-text-primary);
}

.v2-detail-media-dialog::backdrop {
  background: rgb(0 0 0 / 82%);
}

.store-page-safe-html {
  color: var(--arc-text-secondary);
  font-size: 13px;
  line-height: 1.7;
}

.store-page-safe-html > :first-child {
  margin-top: 0;
}

.store-page-safe-html > :last-child {
  margin-bottom: 0;
}

.v2-detail-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
  gap: var(--v2-space-4);
  align-items: start;
}

.v2-detail-main-column {
  display: grid;
  gap: 22px;
  min-width: 0;
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

.v2-detail-seller-identity {
  display: flex;
  align-items: center;
  gap: 10px;
}

.v2-detail-seller-identity h3 {
  margin: 0;
}

.v2-detail-seller-avatar {
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  border-radius: 50%;
  object-fit: cover;
  background: var(--v2-primary-container);
  color: var(--v2-on-primary-container);
  font-weight: 800;
}

.v2-detail-publisher-verification {
  margin: 2px 0 0;
  color: var(--arc-info);
  font-size: 10.5px;
  font-weight: 600;
}

.v2-library-grid,
.v2-social-grid {
  display: grid;
  gap: var(--v2-space-4);
}

.v2-tab-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: var(--v2-space-3);
}

.v2-tab {
  min-height: var(--arc-control-sm);
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-sm);
  background: transparent;
  color: var(--arc-text-muted);
  padding: 8px 14px;
  font-family: var(--arc-font-mono);
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.5px;
  text-transform: uppercase;
}

.v2-tab:hover:not(:disabled) {
  border-color: var(--arc-border-strong);
  color: var(--arc-text-primary);
}

.v2-tab.active,
.v2-tab[aria-selected="true"],
.v2-tab[aria-pressed="true"] {
  color: var(--arc-accent);
  border-color: var(--arc-accent);
  background: oklch(0.78 0.16 60 / 14%);
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

.arc-library-page {
  display: grid;
  gap: 18px;
  min-width: 0;
}

.arc-library-page .arc-page-header {
  min-height: 34px;
}

.arc-library-page .arc-page-heading h1 {
  font-size: 22px;
}

.arc-library-summary,
.arc-library-result-count {
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 600;
}

.arc-library-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
}

.arc-library-search {
  width: min(420px, 100%);
  min-height: 34px;
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-xs);
  padding: 0 10px;
  background: var(--arc-surface-recessed);
}

.arc-library-search:focus-within {
  border-color: var(--arc-focus-ring);
  outline: 1px solid var(--arc-focus-ring);
}

.arc-library-search .material-symbols-outlined {
  color: var(--arc-text-muted);
  font-size: 16px;
}

.arc-library-search input {
  width: 100%;
  min-width: 0;
  border: 0;
  outline: 0;
  padding: 7px 0;
  background: transparent;
  color: var(--arc-text-primary);
  font: 11.5px/1.3 var(--arc-font-mono);
}

.arc-library-toolbar > .v2-btn-secondary {
  min-height: 34px;
  margin-left: auto;
}

.arc-library-result-count {
  margin: -8px 0 0;
}

.arc-library-notice {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  border: 1px solid var(--arc-border-default);
  border-left-color: var(--arc-warning);
  border-radius: var(--arc-radius-xs);
  padding: 10px 12px;
  background: var(--arc-surface-recessed);
}

.arc-library-notice > .material-symbols-outlined {
  color: var(--arc-warning);
  font-size: 17px;
}

.arc-library-notice strong,
.arc-library-notice p {
  font-size: 11px;
  line-height: 1.5;
}

.arc-library-notice p {
  margin: 2px 0 0;
  color: var(--arc-text-muted);
}

.arc-library-notice > .v2-btn-secondary {
  min-height: 32px;
  margin-left: auto;
  padding: 6px 11px;
}

.arc-library-sections,
.arc-library-sections > section,
.arc-library-list {
  display: grid;
  gap: 10px;
}

.arc-library-sections {
  gap: 20px;
}

.arc-library-sections .v2-store-kicker {
  margin: 0 0 2px;
  color: var(--arc-accent);
}

.arc-library-row {
  min-width: 0;
  display: grid;
  grid-template-columns: 56px minmax(180px, 1fr) minmax(210px, auto) auto;
  align-items: center;
  gap: 16px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
}

.arc-library-row:hover {
  border-color: var(--arc-border-strong);
}

.arc-library-art {
  width: 56px;
  height: 56px;
  overflow: hidden;
  border-radius: var(--arc-radius-sm);
  background: var(--arc-surface-artwork);
}

.arc-library-art .arc-game-artwork {
  min-height: 0;
}

.arc-library-art .arc-artwork-fallback {
  padding: 4px;
  font-size: 7.5px;
  letter-spacing: 0.25px;
}

.arc-library-row-copy {
  min-width: 0;
}

.arc-library-row-copy h2,
.arc-library-row-copy p,
.arc-library-row-copy small {
  margin: 0;
}

.arc-library-row-copy h2 {
  overflow: hidden;
  color: var(--arc-text-primary);
  font-size: 14px;
  line-height: 1.25;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-library-row-copy p,
.arc-library-row-copy small {
  display: block;
  margin-top: 2px;
  overflow: hidden;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-library-row-states {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 6px;
}

.arc-library-row-action {
  display: flex;
  justify-content: flex-end;
}

.arc-library-row-action .v2-btn-primary {
  min-width: 96px;
  min-height: 36px;
  padding: 8px 13px;
  white-space: nowrap;
}

.arc-library-action-unavailable,
.arc-library-row-warning,
.arc-library-device-note {
  color: var(--arc-text-muted);
  font-size: 10px;
  line-height: 1.45;
}

.arc-library-row-warning {
  display: block;
  margin-top: 4px;
  color: var(--arc-warning);
}

.arc-library-technical {
  margin-top: 5px;
  color: var(--arc-text-muted);
  font-size: 10px;
}

.arc-library-technical summary {
  width: fit-content;
  cursor: pointer;
}

.arc-library-technical dl {
  display: grid;
  gap: 5px;
  margin: 8px 0 0;
}

.arc-library-technical dl > div {
  display: grid;
  grid-template-columns: 110px minmax(0, 1fr);
  gap: 8px;
}

.arc-library-technical dt {
  color: var(--arc-text-muted);
}

.arc-library-technical dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.arc-library-device-note {
  margin: 0;
}

@media (max-width: 820px) {
  .arc-library-row {
    grid-template-columns: 56px minmax(0, 1fr) auto;
  }

  .arc-library-row-states {
    grid-column: 2 / 3;
    justify-content: flex-start;
  }

  .arc-library-row-action {
    grid-column: 3;
    grid-row: 1 / span 2;
  }
}

@media (max-width: 700px) {
  .arc-library-toolbar {
    align-items: stretch;
    flex-direction: column;
  }

  .arc-library-search {
    width: 100%;
  }

  .arc-library-toolbar > .v2-btn-secondary {
    margin-left: 0;
  }

  .arc-library-row {
    grid-template-columns: 56px minmax(0, 1fr);
  }

  .arc-library-row-states,
  .arc-library-row-action {
    grid-column: 1 / -1;
    grid-row: auto;
    justify-content: flex-start;
  }
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
  border: 1px solid oklch(0.6 0.18 25 / 45%);
  background: oklch(0.6 0.18 25 / 12%);
  color: oklch(0.75 0.18 25);
}

.v2-btn-danger:hover:not(:disabled) {
  border-color: var(--v2-danger);
  background: oklch(0.6 0.18 25 / 20%);
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

.v2-settings-account-row .v2-blossom-health-online,
.v2-settings-account-row .v2-blossom-health-online .v2-blossom-health-dot {
  color: var(--v2-success);
}

.v2-settings-account-row .v2-blossom-health-slow,
.v2-settings-account-row .v2-blossom-health-slow .v2-blossom-health-dot {
  color: var(--v2-warning);
}

.v2-settings-account-row .v2-blossom-health-offline,
.v2-settings-account-row .v2-blossom-health-offline .v2-blossom-health-dot {
  color: var(--v2-danger);
}

.v2-blossom-health-dot {
  background: currentColor;
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

.arc-community-page,
.arc-profile-page,
.arc-profile-main,
.arc-profile-side,
.arc-profile-section {
  display: grid;
  gap: 18px;
  min-width: 0;
}

.arc-profile-page-title {
  margin: 0 0 18px;
  font-size: 22px;
}

.arc-profile-header {
  display: grid;
  grid-template-columns: 70px minmax(0, 1fr);
  align-items: center;
  gap: 18px;
}

.arc-profile-avatar,
.arc-profile-avatar img,
.arc-profile-avatar-fallback {
  width: 70px;
  height: 70px;
  border-radius: 50%;
}

.arc-profile-avatar {
  overflow: hidden;
  background: var(--arc-surface-artwork);
}

.arc-profile-avatar img {
  display: block;
  object-fit: cover;
}

.arc-profile-avatar-fallback {
  display: grid;
  place-items: center;
  background: linear-gradient(135deg, #684f86, #314c86);
  color: #fff;
  font-size: 24px;
  font-weight: 800;
}

.arc-profile-identity,
.arc-profile-title-row > div:first-child {
  min-width: 0;
}

.arc-profile-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
}

.arc-profile-title-row h1 {
  margin: 2px 0 0;
  color: var(--arc-text-primary);
  font-size: 20px;
  line-height: 1.2;
}

.arc-profile-actions {
  flex: 0 0 auto;
}

.arc-profile-identity-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
}

.arc-profile-username,
.arc-profile-key,
.arc-profile-muted,
.arc-profile-readonly {
  margin: 4px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.5;
  overflow-wrap: anywhere;
}

.arc-profile-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 290px;
  align-items: start;
  gap: 22px;
}

.arc-profile-side {
  position: sticky;
  top: calc(var(--arc-shell-height) + 18px);
}

.arc-profile-about,
.arc-profile-section {
  min-width: 0;
}

.arc-profile-about > p:not(.v2-store-kicker),
.arc-profile-unavailable > p:last-child {
  max-width: 72ch;
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 12.5px;
  line-height: 1.65;
}

.arc-profile-metadata {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 18px;
  margin: 12px 0 0;
}

.arc-profile-metadata > div {
  display: flex;
  gap: 7px;
  font-size: 11px;
}

.arc-profile-metadata dt {
  color: var(--arc-text-muted);
}

.arc-profile-metadata dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.arc-profile-metadata a {
  color: var(--arc-info);
}

.arc-profile-key-details {
  margin-top: 10px;
  color: var(--arc-text-muted);
  font-size: 10.5px;
}

.arc-profile-key-details summary {
  width: fit-content;
  cursor: pointer;
}

.arc-profile-key-details p {
  margin: 7px 0 0;
  overflow-wrap: anywhere;
}

.arc-profile-readonly {
  margin-top: 10px;
}

.arc-profile-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.arc-profile-section-header h2,
.arc-profile-unavailable h2 {
  margin: 2px 0 0;
  font-size: 15px;
}

.arc-profile-listings-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
}

.arc-profile-listing-card {
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 0;
  background: var(--arc-surface);
  color: var(--arc-text-primary);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.arc-profile-listing-card:hover {
  border-color: var(--arc-border-strong);
}

.arc-profile-listing-card:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.arc-profile-listing-art {
  width: 100%;
  height: 100px;
}

.arc-profile-listing-copy {
  min-width: 0;
  display: grid;
  gap: 5px;
  padding: 10px 12px 12px;
}

.arc-profile-listing-copy strong {
  overflow: hidden;
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-profile-listing-copy small {
  color: var(--arc-text-muted);
  font-size: 10px;
}

.arc-profile-listing-copy .arc-status-chip {
  width: fit-content;
}

.arc-profile-notice {
  display: flex;
  align-items: center;
  gap: 8px;
  border-left: 2px solid var(--arc-warning);
  padding: 9px 11px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-muted);
  font-size: 11px;
}

.arc-profile-notice strong {
  color: var(--arc-text-primary);
}

.arc-profile-notice-error {
  border-left-color: var(--arc-error);
}

.arc-profile-unavailable {
  border: 1px solid var(--arc-border-default);
  border-left-color: var(--arc-warning);
  border-radius: var(--arc-radius-sm);
  padding: 14px 16px;
  background: var(--arc-surface-recessed);
}

.nip05-badge-container {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 7px;
}

.nip05-badge {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-xs);
  padding: 4px 8px;
  font-size: 10.5px;
  font-weight: 700;
  line-height: 1.25;
}

.nip05-badge-verified {
  border-color: var(--arc-success);
  color: var(--arc-success);
}

.nip05-badge-verifying {
  border-color: var(--arc-info);
  color: var(--arc-info);
}

.nip05-badge-failed {
  border-color: var(--arc-error);
  color: var(--arc-error);
}

.nip05-badge-unverified {
  color: var(--arc-text-muted);
}

.nip05-badge-message {
  flex-basis: 100%;
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 10px;
  line-height: 1.45;
}

.nip05-badge-verify-btn {
  min-height: 30px;
  padding: 5px 9px;
}

@media (max-width: 820px) {
  .arc-profile-layout {
    grid-template-columns: 1fr;
  }

  .arc-profile-side {
    position: static;
  }
}

@media (max-width: 620px) {
  .arc-profile-title-row {
    align-items: flex-start;
    flex-direction: column;
  }

  .arc-profile-listings-grid {
    grid-template-columns: 1fr;
  }
}

.arc-community-page {
  gap: 18px;
}

.v2-achievements {
  gap: 18px;
}

.v2-achievements-hero {
  min-height: 0;
  align-items: center;
  gap: 14px;
  border: 0;
  padding: 0;
  background: transparent;
  box-shadow: none;
}

.v2-achievements-hero-mark {
  width: 48px;
  height: 48px;
  border-radius: var(--arc-radius-md);
  box-shadow: none;
}

.v2-achievements-hero-mark .material-symbols-outlined {
  font-size: 26px;
}

.v2-achievements-hero h1 {
  margin: 2px 0 4px;
  font-size: 22px;
  line-height: 1.2;
}

.v2-achievements-hero > div:last-child > p:last-child {
  font-size: 11px;
  line-height: 1.5;
}

.v2-achievement-results {
  display: grid;
  gap: 12px;
}

.v2-achievement-partial {
  display: flex;
  align-items: center;
  gap: 8px;
  border-left: 2px solid var(--arc-warning);
  padding: 9px 11px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-muted);
  font-size: 10.5px;
}

.v2-achievement-partial strong {
  color: var(--arc-text-primary);
}

.v2-achievement-partial > div {
  display: flex;
  align-items: center;
  gap: 8px;
}

.v2-achievement-partial .v2-btn-secondary {
  min-height: 30px;
  margin-left: auto;
  padding: 5px 9px;
}

.v2-achievement-grid {
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 14px;
}

.v2-achievement-card {
  grid-template-rows: auto auto 1fr;
  gap: 10px;
  border-radius: var(--arc-radius-md);
  padding: 16px;
  background: var(--arc-surface);
  box-shadow: none;
  text-align: center;
}

.v2-achievement-art {
  width: 48px;
  height: 48px;
  min-height: 0;
  justify-self: center;
  border-radius: 10px;
}

.v2-achievement-art .arc-game-artwork {
  min-height: 0;
}

.v2-achievement-art .arc-artwork-fallback {
  padding: 3px;
  font-size: 6.5px;
  letter-spacing: 0;
}

.v2-achievement-copy h2 {
  margin: 2px 0 5px;
  font: 700 12px/1.3 var(--arc-font-mono);
}

.v2-achievement-copy > p:last-child {
  font-size: 10px;
  line-height: 1.45;
}

.v2-achievement-meta {
  gap: 5px;
  padding-top: 8px;
  text-align: left;
}

.v2-achievement-meta > div {
  gap: 6px;
}

.v2-achievement-meta dt,
.v2-achievement-meta dd {
  font-size: 9px;
}

.v2-achievement-state {
  min-height: 220px;
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
}

.v2-badge-showcase {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 16px;
  background: var(--arc-surface);
}

.v2-badge-showcase-header {
  gap: 9px;
  margin-bottom: 10px;
}

.v2-badge-showcase-header > .material-symbols-outlined {
  width: 34px;
  height: 34px;
  border-radius: var(--arc-radius-sm);
}

.v2-badge-showcase-header h2 {
  font-size: 14px;
}

.v2-badge-showcase-source {
  margin: 0 0 10px;
  color: var(--arc-text-muted);
  font-size: 10px;
  line-height: 1.45;
}

.v2-badge-showcase-warning {
  margin: 0 0 10px;
  border-left: 2px solid var(--arc-warning);
  padding-left: 8px;
  color: var(--arc-text-secondary);
  font-size: 10px;
  line-height: 1.45;
}

.v2-badge-showcase-row {
  gap: 8px;
}

.v2-badge-chip {
  gap: 9px;
  border-radius: var(--arc-radius-sm);
  padding: 8px;
  background: var(--arc-surface-recessed);
}

.v2-badge-chip-art {
  width: 44px;
  height: 44px;
  border-radius: var(--arc-radius-sm);
}

.v2-badge-chip-art .arc-game-artwork {
  min-height: 0;
}

.v2-badge-chip-art .arc-artwork-fallback {
  padding: 3px;
  font-size: 6px;
  letter-spacing: 0;
}

.v2-badge-chip strong {
  font-size: 11px;
}

.v2-badge-chip span {
  font-size: 9.5px;
}

@media (max-width: 924px) {
  .v2-achievement-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (max-width: 650px) {
  .v2-achievement-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .v2-achievement-partial {
    align-items: flex-start;
    flex-direction: column;
  }


  .v2-achievement-partial > div {
    align-items: flex-start;
    flex-direction: column;
  }

  .v2-achievement-partial .v2-btn-secondary {
    margin-left: 0;
  }
}

@media (max-height: 640px) {
  .arc-profile-side {
    position: static;
  }
}

.v2-display {
  font-family: var(--v2-font-display);
  letter-spacing: -0.02em;
}

.v2-panel,
.arc-clipped-panel {
  background: var(--arc-surface);
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-lg);
}

.arc-clipped-panel {
  clip-path: var(--arc-card-clip);
  border-color: var(--arc-border-card);
  border-radius: 0;
}

.v2-panel-glass {
  background: var(--arc-surface);
  border: 1px solid var(--arc-border-subtle);
  box-shadow: none;
  border-radius: var(--arc-radius-lg);
}

.v2-btn-primary,
.v2-btn-secondary,
.v2-btn-ghost,
.v2-btn-danger,
.arc-icon-button {
  min-height: var(--arc-control-md);
  padding: 8px 16px;
  border-radius: var(--arc-radius-sm);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--arc-space-2);
  font-family: var(--arc-font-mono);
  font-size: 12px;
  font-weight: 800;
  line-height: 1.15;
  text-align: center;
  cursor: pointer;
  transition: background-color 120ms ease, border-color 120ms ease, color 120ms ease,
    opacity 120ms ease, transform 120ms ease;
}

.v2-btn-primary {
  border: 1px solid var(--arc-accent);
  background: var(--arc-accent);
  color: var(--arc-background);
}

.v2-btn-primary:hover:not(:disabled) {
  background: oklch(0.83 0.16 60);
  border-color: oklch(0.83 0.16 60);
}

.v2-btn-secondary {
  border: 1px solid oklch(0.55 0.13 195);
  background: transparent;
  color: var(--arc-action);
}

.v2-btn-secondary:hover:not(:disabled) {
  border-color: var(--arc-action);
  background: oklch(0.8 0.14 195 / 10%);
}

.v2-btn-ghost,
.arc-icon-button {
  border: 1px solid transparent;
  background: transparent;
  color: var(--arc-text-secondary);
}

.v2-btn-ghost:hover:not(:disabled),
.arc-icon-button:hover:not(:disabled) {
  border-color: var(--arc-border-default);
  color: var(--arc-text-primary);
}

.arc-icon-button {
  width: var(--arc-control-sm);
  min-height: var(--arc-control-sm);
  padding: 0;
}

.v2-btn-primary:focus-visible,
.v2-btn-secondary:focus-visible,
.v2-btn-ghost:focus-visible,
.v2-btn-danger:focus-visible,
.arc-icon-button:focus-visible,
.v2-tab:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.v2-btn-primary:active:not(:disabled),
.v2-btn-secondary:active:not(:disabled),
.v2-btn-ghost:active:not(:disabled),
.v2-btn-danger:active:not(:disabled),
.arc-icon-button:active:not(:disabled) {
  transform: translateY(1px);
}

.v2-btn-primary:disabled,
.v2-btn-secondary:disabled,
.v2-btn-ghost:disabled,
.v2-btn-danger:disabled,
.arc-icon-button:disabled {
  border-color: var(--arc-disabled-background);
  background: var(--arc-disabled-background);
  color: var(--arc-text-disabled);
  cursor: not-allowed;
  opacity: 1;
}

.v2-input {
  width: 100%;
  min-height: var(--arc-control-md);
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-sm);
  background: var(--arc-surface-recessed);
  color: var(--arc-text-primary);
  padding: 8px 10px;
  font-family: var(--arc-font-mono);
}

.v2-input::placeholder {
  color: oklch(0.52 0.01 60);
}

.v2-input.v2-topbar-search {
  min-height: 0;
  padding: 0;
  border: none;
  background: transparent;
}

.v2-input.v2-topbar-search::placeholder {
  color: oklch(0.52 0.01 60);
  text-indent: 0;
}

.v2-input:focus,
.v2-input:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 1px;
  border-color: var(--arc-focus-ring);
  box-shadow: none;
}

/* Phase 2 shared navigation and feedback primitives. */
.arc-page-tabs,
.v2-tab-row {
  min-width: 0;
  display: flex;
  align-items: stretch;
  gap: 8px;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: thin;
}

.arc-page-tab,
.v2-tab {
  position: relative;
  min-height: 34px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-bottom: 1px solid transparent;
  border-radius: 0;
  padding: 8px 14px;
  background: transparent;
  color: var(--arc-text-muted);
  font-family: var(--arc-font-mono);
  font-size: 11.5px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0;
  text-decoration: none;
  text-transform: uppercase;
  white-space: nowrap;
  cursor: pointer;
}

.arc-publisher-tabs .arc-page-tab {
  font-size: 11px;
}

.arc-publisher-tabs .arc-page-tabs {
  flex-wrap: wrap;
  overflow: visible;
}

.arc-publisher-tabs .arc-tab-unavailable > span:first-child {
  display: none;
}

.arc-publisher-shell {
  display: grid;
  gap: 18px;
}

.arc-page-tab:hover:not(:disabled):not([aria-disabled="true"]),
.v2-tab:hover:not(:disabled) {
  color: var(--arc-text-primary);
}

.arc-page-tab-active,
.arc-page-tab[aria-current="page"],
.arc-page-tab[aria-selected="true"],
.arc-page-tab[aria-pressed="true"],
.v2-tab.active,
.v2-tab[aria-selected="true"],
.v2-tab[aria-pressed="true"] {
  border-bottom-color: var(--arc-accent);
  background: transparent;
  color: var(--arc-accent);
}

.arc-page-tab:focus-visible,
.v2-tab:focus-visible {
  border-radius: var(--arc-radius-xs);
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: -2px;
}

.arc-page-tab:disabled,
.arc-page-tab[aria-disabled="true"],
.v2-tab:disabled {
  gap: 5px;
  color: var(--arc-text-disabled);
  cursor: not-allowed;
  opacity: 0.48;
}

.arc-tab-disabled-icon {
  font-size: 11px;
}

.arc-tab-unavailable {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-xs);
  padding: 2px 4px;
  font-size: 8px;
  letter-spacing: 0.4px;
}

.arc-status-chip {
  max-width: 100%;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-xs);
  background: transparent;
  color: var(--arc-text-secondary);
  font-family: var(--arc-font-mono);
  font-weight: 800;
  line-height: 1;
  letter-spacing: 0.35px;
  text-transform: uppercase;
  overflow-wrap: anywhere;
  text-align: left;
  white-space: normal;
}

.arc-status-chip .material-symbols-outlined {
  font-size: 13px;
}

.arc-status-compact {
  min-height: 22px;
  padding: 5px 9px;
  font-size: 10.5px;
}

.arc-status-standard {
  min-height: 27px;
  padding: 6px 12px;
  font-size: 11.5px;
}

.arc-status-active {
  border-color: var(--arc-accent);
  background: var(--arc-accent);
  color: var(--arc-background);
}

.arc-status-success,
.arc-status-owned,
.arc-status-installed,
.arc-status-published,
.arc-status-verified {
  border-color: var(--arc-success);
  background: oklch(0.72 0.16 145 / 14%);
  color: oklch(0.78 0.16 145);
}

.arc-status-warning,
.arc-status-update {
  border-color: var(--arc-warning);
  background: oklch(0.7 0.15 60 / 14%);
  color: oklch(0.78 0.15 60);
}

.arc-status-error,
.arc-status-unverified {
  border-color: var(--arc-error);
  background: oklch(0.6 0.18 25 / 12%);
  color: oklch(0.75 0.18 25);
}

.arc-status-pending,
.arc-status-downloading,
.arc-status-draft {
  border-color: var(--arc-info);
  background: oklch(0.68 0.15 300 / 14%);
  color: oklch(0.8 0.15 300);
}

.arc-status-public {
  border-color: var(--arc-action);
  background: oklch(0.8 0.14 195 / 16%);
  color: var(--arc-action);
}

.arc-status-timed {
  border-color: var(--arc-info);
  background: oklch(0.68 0.15 300 / 16%);
  color: oklch(0.8 0.15 300);
}

.arc-status-gated,
.arc-status-unavailable,
/* Ended and Cancelled are both terminal, but they are different outcomes and
   must not read as the same chip. --arc-text-subdued was also too dark for
   10.5px chip text on the card surface (~2.5:1). */
.arc-status-expired,
.arc-status-cancelled {
  border-color: var(--arc-border-default);
  background: rgb(255 255 255 / 4%);
  color: var(--arc-text-muted);
}

.arc-status-cancelled {
  border-style: dashed;
  border-color: oklch(0.6 0.18 25 / 55%);
  color: oklch(0.74 0.09 25);
}

.arc-status-neutral {
  border-color: var(--arc-border-default);
  color: var(--arc-text-secondary);
}

.arc-game-artwork {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: var(--arc-surface-artwork);
}

.arc-game-artwork img {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: cover;
}

.arc-artwork-fallback {
  width: 100%;
  height: 100%;
  display: grid;
  place-items: center;
  background: repeating-linear-gradient(135deg, #1b1b1d 0 10px, #141417 10px 20px);
  color: oklch(0.5 0.01 60);
  font-family: var(--arc-font-mono);
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.75px;
  text-align: center;
}

.arc-artwork-loading {
  color: var(--arc-text-muted);
}

.arc-game-card {
  position: relative;
  min-width: 0;
  height: 100%;
  overflow: hidden;
  clip-path: var(--arc-card-clip);
  border: 1px solid var(--arc-border-card);
  background: var(--arc-surface);
  transition: border-color 120ms ease, background-color 120ms ease, transform 120ms ease;
}

.arc-game-card:hover {
  border-color: var(--arc-border-strong);
  background: var(--arc-surface-recessed);
}

.arc-game-card-main {
  width: 100%;
  min-width: 0;
  display: block;
  border: 0;
  padding: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.arc-game-card-main:focus-visible,
.arc-game-card-favorite:focus-visible,
.arc-game-card-action:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: -3px;
}

.arc-game-card-main:disabled,
.arc-game-card-disabled {
  cursor: not-allowed;
  opacity: 0.62;
}

.arc-game-card-art {
  position: relative;
  width: 100%;
  height: 160px;
  overflow: hidden;
  background: var(--arc-surface-artwork);
}

.arc-game-card-browse .arc-game-card-art {
  height: 150px;
}

.arc-game-card-browse .arc-game-card-title {
  font-size: 16px;
}

.arc-artwork-hero {
  min-height: 180px;
}

.arc-artwork-thumbnail {
  aspect-ratio: 16 / 10;
}

.arc-game-card-art-shade {
  position: absolute;
  inset: auto 0 0;
  height: 72px;
  background: linear-gradient(180deg, transparent, rgb(0 0 0 / 86%));
  pointer-events: none;
}

.arc-game-card-title {
  position: absolute;
  left: 14px;
  right: 14px;
  bottom: 12px;
  overflow: hidden;
  color: #fff;
  font-family: var(--arc-font-mono);
  font-size: 17px;
  font-weight: 800;
  line-height: 1.15;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-game-card-body {
  min-width: 0;
  display: grid;
  gap: 7px;
  padding: 12px 14px;
}

.arc-game-card-meta-row {
  min-width: 0;
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.arc-game-card-publisher,
.arc-game-card-metadata {
  min-width: 0;
  overflow: hidden;
  color: oklch(0.62 0.01 60);
  font-size: 10.5px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-game-card-price {
  flex: 0 0 auto;
  color: var(--arc-accent);
  font-size: 11px;
  font-weight: 800;
}

.arc-game-card-summary {
  display: -webkit-box;
  overflow: hidden;
  color: var(--arc-text-muted);
  font-size: 11px;
  line-height: 1.55;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.arc-game-card-statuses {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}

.arc-game-card-favorite {
  position: absolute;
  top: 10px;
  left: 10px;
  z-index: 2;
  width: 30px;
  height: 30px;
  display: grid;
  place-items: center;
  border: 1px solid rgb(255 255 255 / 18%);
  border-radius: 50%;
  background: var(--arc-overlay-favorite);
  color: var(--arc-text-primary);
  cursor: pointer;
}

.arc-game-card-favorite .material-symbols-outlined {
  font-size: 16px;
}

.arc-game-card-favorite-active {
  color: var(--arc-accent);
}

.arc-game-card-action {
  width: calc(100% - 28px);
  min-height: 30px;
  margin: 0 14px 12px;
  border: 1px solid var(--arc-accent);
  border-radius: var(--arc-radius-sm);
  padding: 8px 12px;
  background: var(--arc-accent);
  color: var(--arc-background);
  font-family: var(--arc-font-mono);
  font-size: 11px;
  font-weight: 800;
  cursor: pointer;
}

.arc-game-card-action:disabled {
  border-color: var(--arc-disabled-background);
  background: var(--arc-disabled-background);
  color: var(--arc-text-disabled);
  cursor: not-allowed;
}

.arc-feedback {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 14px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-lg);
  background: var(--arc-surface);
  color: var(--arc-text-secondary);
}

.arc-feedback-inline {
  border: 0;
  padding: 0;
  background: transparent;
}

.arc-feedback-compact,
.arc-relay-feedback {
  padding: 12px 14px;
  border-radius: var(--arc-radius-md);
}

.arc-feedback-panel {
  min-height: 136px;
  padding: 24px;
}

.arc-feedback-full {
  min-height: 280px;
  justify-content: center;
  padding: 60px 28px;
  text-align: center;
}

.arc-feedback-empty {
  flex-direction: column;
  justify-content: center;
  padding-top: 60px;
  padding-bottom: 60px;
  text-align: center;
}

.arc-feedback-copy {
  min-width: 0;
}

.arc-feedback h2 {
  margin: 0;
  color: var(--arc-text-primary);
  font-size: 13px;
  font-weight: 800;
}

.arc-feedback p {
  margin: 5px 0 0;
  color: var(--arc-text-muted);
  font-size: 12px;
  line-height: 1.55;
}

.arc-feedback small {
  display: block;
  margin-top: 5px;
  color: var(--arc-text-subdued);
  font-size: 10.5px;
}

.arc-feedback-icon {
  flex: 0 0 auto;
  color: var(--arc-text-muted);
  font-size: 22px;
}

.arc-feedback-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}

.arc-relay-feedback {
  border-color: oklch(0.55 0.13 195 / 45%);
  background: oklch(0.8 0.14 195 / 8%);
}

.arc-relay-feedback .arc-feedback-icon,
.arc-relay-feedback h2 {
  color: var(--arc-action);
}

.arc-relay-feedback-warning {
  border-color: oklch(0.7 0.15 60 / 45%);
  background: oklch(0.7 0.15 60 / 8%);
}

.arc-relay-feedback-warning .arc-feedback-icon,
.arc-relay-feedback-warning h2 {
  color: var(--arc-warning);
}

.arc-feedback-error {
  border-color: oklch(0.6 0.18 25 / 42%);
  background: oklch(0.6 0.18 25 / 10%);
}

.arc-feedback-error .arc-feedback-icon,
.arc-feedback-error h2 {
  color: oklch(0.75 0.18 25);
}

.arc-error-inline {
  border: 0;
  padding: 0;
  background: transparent;
}

.arc-error-panel {
  padding: 18px;
}

.arc-error-full {
  min-height: 280px;
  justify-content: center;
  padding: 60px 28px;
  text-align: center;
}

.arc-error-detail {
  margin-top: 8px;
  color: var(--arc-text-subdued);
  font-size: 10.5px;
}

.arc-error-detail summary {
  cursor: pointer;
}

.arc-inline-loading {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 600;
}

.arc-loading-mark {
  width: 9px;
  height: 9px;
  flex: 0 0 auto;
  border: 1px solid var(--arc-border-strong);
  border-top-color: var(--arc-action);
  border-radius: 50%;
  animation: arc-loading-spin 700ms linear infinite;
}

@keyframes arc-loading-spin {
  to { transform: rotate(360deg); }
}

.arc-skeleton {
  display: block;
  border-radius: var(--arc-radius-sm);
  background: linear-gradient(90deg, var(--arc-progress-track), rgb(255 255 255 / 8%), var(--arc-progress-track));
  background-size: 200% 100%;
  animation: arc-skeleton-shift 900ms ease-in-out infinite;
}

@keyframes arc-skeleton-shift {
  to { background-position: -200% 0; }
}

.arc-skeleton-text {
  width: 100%;
  height: 10px;
}

.arc-skeleton-panel {
  width: 100%;
  min-height: 120px;
}

.arc-skeleton-card {
  width: 100%;
  min-height: 160px;
}

.arc-game-card-skeleton {
  min-height: 278px;
}

.arc-game-card-skeleton-art {
  position: absolute;
  inset: 0;
  height: 160px;
  border-radius: 0;
}

.arc-game-card-skeleton-title {
  position: absolute;
  left: 14px;
  right: 28%;
  bottom: 14px;
  z-index: 1;
  width: auto;
  height: 15px;
}

.arc-game-card-skeleton-copy {
  display: grid;
  gap: 10px;
  padding: 14px;
}

.arc-skeleton-short {
  width: 42%;
}

.arc-skeleton-chip {
  width: 34%;
  height: 22px;
}

.arc-game-card-skeleton-action {
  width: calc(100% - 28px);
  min-height: 30px;
  height: 30px;
  margin: 0 14px 12px;
}

.v2-btn-action,
.v2-btn-neutral,
.v2-btn-success {
  min-height: var(--arc-control-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--arc-space-2);
  border-radius: var(--arc-radius-sm);
  padding: 8px 16px;
  font-family: var(--arc-font-mono);
  font-size: 12px;
  font-weight: 800;
  line-height: 1.15;
  cursor: pointer;
}

.v2-btn-action {
  border: 1px solid var(--arc-action);
  background: var(--arc-action);
  color: var(--arc-background);
}

.v2-btn-neutral {
  border: 1px solid var(--arc-border-strong);
  background: transparent;
  color: var(--arc-text-primary);
}

.v2-btn-success {
  border: 1px solid var(--arc-success);
  background: var(--arc-success);
  color: var(--arc-background);
}

.v2-btn-action:focus-visible,
.v2-btn-neutral:focus-visible,
.v2-btn-success:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.v2-btn-action:disabled,
.v2-btn-neutral:disabled,
.v2-btn-success:disabled {
  border-color: var(--arc-disabled-background);
  background: var(--arc-disabled-background);
  color: var(--arc-text-disabled);
  cursor: not-allowed;
}

.arc-btn-compact {
  min-height: 30px;
  padding: 7px 12px;
  font-size: 11px;
}

.arc-btn-large {
  min-height: 42px;
  padding: 11px 20px;
  font-size: 13px;
}

.arc-btn-clipped {
  clip-path: polygon(0 0, calc(100% - 7px) 0, 100% 7px, 100% 100%, 0 100%);
  border-radius: 0;
}

.arc-btn-content {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: inherit;
}

.arc-btn-content-hidden {
  visibility: hidden;
}

.arc-btn-busy {
  position: absolute;
  display: inline-flex;
  align-items: center;
  gap: 7px;
}

.v2-btn-primary,
.v2-btn-secondary,
.v2-btn-ghost,
.v2-btn-danger,
.v2-btn-action,
.v2-btn-neutral,
.v2-btn-success {
  position: relative;
}

@media (prefers-reduced-motion: reduce) {
  .arc-loading-mark,
  .arc-skeleton {
    animation: none;
  }
}

/* Phase 3 Store Home and Browse composition. */
.arc-store-home {
  min-width: 0;
  margin: calc(-1 * var(--arc-page-block)) calc(-1 * var(--arc-page-inline)) -60px;
}

.arc-store-hero {
  width: 100%;
  height: 380px;
  min-height: 380px;
  display: grid;
  grid-template-columns: 38% 62%;
  border: 0;
  border-bottom: 1px solid var(--arc-separator-strong);
  background: var(--arc-surface);
  clip-path: polygon(0 0, calc(100% - 12px) 0, 100% 12px, 100% 100%, 12px 100%, 0 calc(100% - 12px));
}

.arc-store-hero:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: -3px;
}

.arc-store-hero-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 8px;
  overflow: hidden;
  padding: 26px 30px;
  border-right: 1px solid var(--arc-separator-strong);
}

.arc-store-hero-kicker,
.arc-store-section-heading {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  color: var(--arc-accent);
  font-size: 10.5px;
  font-weight: 800;
  letter-spacing: 2px;
  text-transform: uppercase;
}

.arc-store-hero-kicker i,
.arc-store-section-heading i {
  min-width: 18px;
  flex: 1 1 auto;
  border-top: 1px dashed var(--arc-separator-strong);
}

.arc-store-hero-copy h1 {
  max-width: 100%;
  margin: 4px 0 0;
  overflow-wrap: anywhere;
  color: var(--arc-text-heading);
  font-family: var(--arc-font-mono);
  font-size: 32px;
  font-weight: 800;
  line-height: 1.08;
  letter-spacing: 0.5px;
}

.arc-store-hero-publisher {
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 11px;
  font-weight: 600;
  line-height: 1.4;
}

.arc-store-hero-statuses {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.arc-store-hero-tag {
  min-height: 27px;
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-sm);
  padding: 6px 11px;
  color: var(--arc-text-secondary);
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
}

.arc-store-hero-summary {
  max-width: 400px;
  margin: 2px 0 0;
  display: -webkit-box;
  overflow: hidden;
  color: var(--arc-text-secondary);
  font-size: 12.5px;
  font-weight: 400;
  line-height: 1.55;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
}

.arc-store-hero-actions {
  margin-top: auto;
  display: flex;
  flex-wrap: nowrap;
  align-items: center;
  gap: 8px;
}

.arc-store-hero-actions > button {
  min-height: 38px;
  padding: 11px 14px;
  font-size: 13px;
}

.arc-store-hero-capabilities {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 700;
}

.arc-store-hero-capabilities span {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.arc-store-hero-capabilities .material-symbols-outlined {
  color: var(--arc-action);
  font-size: 14px;
}

.arc-store-hero-art {
  position: relative;
  min-width: 0;
  min-height: 380px;
  overflow: hidden;
  border-radius: 0;
  background: var(--arc-surface-artwork);
}

.arc-store-hero-art .arc-game-artwork {
  min-height: 380px;
}

.arc-store-hero-art-shade {
  position: absolute;
  inset: auto 0 0;
  height: 128px;
  background: linear-gradient(180deg, transparent, rgb(11 11 13 / 82%));
  pointer-events: none;
}

.arc-store-carousel {
  position: absolute;
  inset: auto 0 0;
  z-index: 2;
  height: 52px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  border-top: 1px solid var(--arc-border-subtle);
  padding: 0 14px 0 16px;
  background: var(--arc-overlay-control);
}

.arc-store-carousel-indicators,
.arc-store-carousel-controls {
  display: flex;
  align-items: center;
  gap: 6px;
}

.arc-store-carousel-indicators {
  max-width: 58%;
  overflow-x: auto;
  scrollbar-width: thin;
}

.arc-store-carousel-indicators button {
  position: relative;
  width: 24px;
  height: 24px;
  flex: 0 0 24px;
  border: 0;
  padding: 0;
  background: transparent;
  cursor: pointer;
}

.arc-store-carousel-indicators button::after {
  content: "";
  position: absolute;
  left: 50%;
  top: 50%;
  width: 14px;
  height: 5px;
  border-radius: 3px;
  background: rgb(255 255 255 / 25%);
  transform: translate(-50%, -50%);
  transition: width 120ms ease, background-color 120ms ease;
}

.arc-store-carousel-indicators button.active::after {
  width: 24px;
  background: var(--arc-accent);
}

.arc-store-carousel-indicators button:focus-visible,
.arc-store-carousel-controls button:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.arc-store-carousel-controls > span {
  margin-right: 4px;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 1px;
}

.arc-store-carousel-controls .arc-icon-button {
  width: 34px;
  min-height: 34px;
  border-color: var(--arc-border-default);
  background: rgb(10 10 12 / 70%);
}

.arc-store-hero-loading {
  background: var(--arc-surface);
}

.arc-store-skeleton-kicker {
  width: 36%;
}

.arc-store-skeleton-title {
  width: 86%;
  height: 34px;
  margin-top: 12px;
}

.arc-store-skeleton-meta {
  width: 58%;
}

.arc-store-skeleton-summary {
  min-height: 92px;
  margin-top: 14px;
}

.arc-store-hero-state {
  min-height: 380px;
  display: grid;
  place-items: center;
  border-bottom: 1px solid var(--arc-separator-strong);
  background: var(--arc-surface-lowest);
}

.arc-store-hero-state > .arc-feedback {
  width: min(620px, calc(100% - 56px));
}

.arc-store-content {
  display: grid;
  gap: 24px;
  padding: 26px 28px 60px;
}

.arc-store-content > .arc-relay-feedback {
  margin-bottom: -6px;
}

.arc-store-enrichment-warning,
.arc-browse-enrichment-status {
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 11px;
  line-height: 1.55;
}

.arc-store-enrichment-warning,
.arc-browse-enrichment-warning {
  color: var(--arc-warning);
}

.arc-store-section,
.arc-store-secondary-section {
  min-width: 0;
}

.arc-store-section-heading {
  margin-bottom: 18px;
  color: var(--arc-text-primary);
  font-size: 12px;
  letter-spacing: 1.5px;
}

.arc-store-section-heading h2 {
  margin: 0;
  font: inherit;
}

.arc-store-section-heading button {
  flex: 0 0 auto;
  border: 0;
  padding: 3px 0;
  background: transparent;
  color: var(--arc-accent);
  font-family: var(--arc-font-mono);
  font-size: 10.5px;
  font-weight: 800;
  text-transform: uppercase;
  cursor: pointer;
}

.arc-store-section-heading button:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.arc-store-grid,
.arc-browse-grid {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  align-items: stretch;
  gap: 18px;
}

.arc-store-promotion {
  width: 100%;
  min-height: 76px;
  display: flex;
  align-items: center;
  gap: 14px;
  border: 1px solid var(--arc-border-card);
  padding: 14px 16px;
  background: var(--arc-surface);
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
  text-align: left;
  cursor: pointer;
}

.arc-store-promotion > .material-symbols-outlined {
  color: var(--arc-info);
  font-size: 24px;
}

.arc-store-promotion > span:last-child {
  min-width: 0;
  display: grid;
  gap: 3px;
}

.arc-store-promotion strong {
  color: var(--arc-info);
  font-size: 10.5px;
  letter-spacing: 1px;
  text-transform: uppercase;
}

.arc-store-promotion b {
  overflow: hidden;
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.arc-store-promotion small {
  color: var(--arc-text-muted);
  font-size: 11px;
}

.arc-store-categories {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.arc-store-categories button,
.arc-browse-category,
.arc-browse-active-filters button {
  min-height: 30px;
  border: 1px solid var(--arc-border-default);
  border-radius: var(--arc-radius-sm);
  padding: 7px 10px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-secondary);
  font-family: var(--arc-font-mono);
  font-size: 10.5px;
  font-weight: 700;
  cursor: pointer;
}

.arc-store-categories button:hover,
.arc-browse-category:hover,
.arc-browse-active-filters button:hover {
  border-color: var(--arc-border-strong);
  color: var(--arc-text-primary);
}

.arc-store-categories button:focus-visible,
.arc-browse-category:focus-visible,
.arc-browse-active-filters button:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.arc-browse {
  min-width: 0;
}

.arc-browse-header {
  margin-bottom: 16px;
}

.arc-browse-header h1 {
  margin: 0;
  color: var(--arc-text-heading);
  font-size: 22px;
  font-weight: 800;
  line-height: 1.15;
}

.arc-browse-toolbar {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  margin-bottom: 14px;
}

.arc-browse-search {
  width: 260px;
  height: var(--arc-control-md);
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid var(--arc-border-default);
  border-radius: 5px;
  padding: 0 10px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-muted);
}

.arc-browse-search .material-symbols-outlined {
  flex: 0 0 auto;
  font-size: 16px;
}

.arc-browse-search input {
  min-width: 0;
  width: 100%;
  height: 100%;
  border: 0;
  padding: 0;
  background: transparent;
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
  font-size: 11.5px;
  outline: none;
}

.arc-browse-search:focus-within,
.arc-browse-filter select:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 1px;
}

.arc-browse-filter select {
  height: var(--arc-control-md);
  border: 1px solid var(--arc-border-default);
  border-radius: 5px;
  padding: 8px 10px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
  font-size: 12px;
  font-weight: 600;
}

.arc-browse-result-count {
  margin: 0 0 0 auto;
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 600;
}

.arc-browse-categories,
.arc-browse-active-filters {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}

.arc-browse-categories > span,
.arc-browse-active-filters > span {
  margin-right: 2px;
  color: var(--arc-text-subdued);
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 1px;
  text-transform: uppercase;
}

.arc-browse-category.active,
.arc-browse-category[aria-pressed="true"] {
  border-color: var(--arc-accent);
  background: oklch(0.78 0.16 60 / 12%);
  color: var(--arc-accent);
}

.arc-browse-active-filters {
  margin-top: -4px;
}

.arc-browse-active-filters button {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  border-color: oklch(0.8 0.14 195 / 45%);
  background: oklch(0.8 0.14 195 / 8%);
  color: var(--arc-action);
}

.arc-browse-active-filters .arc-browse-clear-all {
  border-color: transparent;
  background: transparent;
  color: var(--arc-accent);
}

.arc-browse > .arc-relay-feedback,
.arc-browse-enrichment-status {
  margin-bottom: 16px;
}

.arc-browse-state {
  min-height: 260px;
}

.arc-browse-state > .arc-feedback {
  min-height: 260px;
}

.arc-browse-filter-pending {
  min-height: 160px;
  display: grid;
  place-items: center;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-lg);
  background: var(--arc-surface);
}

.arc-browse-pagination {
  min-height: 56px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding-top: 20px;
}

.arc-browse-exhausted {
  margin: 28px 0 0;
  color: var(--arc-text-muted);
  font-size: 11px;
  text-align: center;
}

@media (max-width: 923px) {
  .arc-store-grid,
  .arc-browse-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .arc-store-hero {
    grid-template-columns: 42% 58%;
  }

  .arc-store-hero-copy {
    padding-inline: 22px;
  }

  .arc-store-hero-copy h1 {
    font-size: 28px;
  }
}

@media (max-width: 720px) {
  .arc-store-grid,
  .arc-browse-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .arc-store-hero {
    height: auto;
    grid-template-columns: 1fr;
  }

  .arc-store-hero-copy {
    min-height: 340px;
    border-right: 0;
    border-bottom: 1px solid var(--arc-separator-strong);
  }

  .arc-store-hero-art,
  .arc-store-hero-art .arc-game-artwork {
    min-height: 320px;
  }

  .arc-browse-result-count {
    width: 100%;
    margin-left: 0;
  }
}

@media (max-width: 520px) {
  .arc-store-grid,
  .arc-browse-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .arc-browse-search {
    width: 100%;
  }
}

@media (prefers-reduced-motion: reduce) {
  .arc-store-carousel-indicators button {
    transition: none;
  }
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

/* The WebKitGTK webview used by the desktop shell renders the contents of a
   closed <details>; keep collapsed publisher disclosures actually collapsed. */
.v2-publisher-studio details:not([open]) > *:not(summary) {
  display: none;
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

/* Phase 7 publisher dashboard; management editors retain their existing density. */
.arc-publisher-tabs .arc-page-tab {
  min-height: 32px;
  border: 1px solid var(--arc-border-strong);
  border-radius: var(--arc-radius-xs);
  padding: 8px 14px;
  font-size: 11px;
}

.arc-publisher-tabs .arc-page-tab-active,
.arc-publisher-tabs .arc-page-tab[aria-current="page"] {
  border-color: var(--arc-accent);
}

.v2-publisher-dashboard {
  width: 100%;
  gap: 14px;
  max-width: none;
  margin: 0;
}

.v2-publisher-dashboard-header {
  align-items: center;
  padding: 0 0 4px;
  overflow: visible;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}

.v2-publisher-dashboard-header h1 {
  margin: 0;
  font: 800 20px/1.2 var(--arc-font-mono);
  letter-spacing: 0;
}

.v2-publisher-dashboard .v2-publisher-kicker {
  margin-bottom: 4px;
  color: var(--arc-text-muted);
  font-size: 10px;
  letter-spacing: 1.5px;
}

.v2-publisher-dashboard .v2-btn-primary,
.v2-publisher-dashboard .v2-btn-secondary {
  min-height: 32px;
  border-radius: var(--arc-radius-xs);
  padding: 7px 12px;
  font-size: 11px;
}

.v2-publisher-dashboard-header .v2-btn-primary {
  padding-inline: 18px;
  font-size: 12.5px;
  font-weight: 800;
  text-transform: uppercase;
}

.v2-publisher-action-requirement {
  margin: -8px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.45;
  text-align: right;
}

.v2-publisher-dashboard-content,
.v2-publisher-game-list,
.v2-publisher-campaign-summary-list {
  display: grid;
  gap: 10px;
}

.v2-publisher-feedback {
  min-height: 190px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 14px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 24px;
  background: var(--arc-surface);
  color: var(--arc-text-secondary);
}

.v2-publisher-feedback > .material-symbols-outlined {
  color: var(--arc-accent);
  font-size: 28px;
}

.v2-publisher-feedback h2,
.v2-publisher-attention h2,
.v2-publisher-campaigns h2,
.v2-publisher-unavailable-metrics h2 {
  margin: 0 0 5px;
  font: 800 14px/1.3 var(--arc-font-mono);
}

.v2-publisher-feedback p,
.v2-publisher-unavailable-metrics p {
  max-width: 70ch;
  margin: 0;
  font-size: 11.5px;
  line-height: 1.6;
}

.v2-publisher-feedback .v2-btn-primary {
  margin-top: 12px;
}

.v2-publisher-feedback-error {
  border-color: color-mix(in oklch, var(--arc-error) 50%, transparent);
}

.v2-publisher-facts {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin: 0;
  border-block: 1px solid var(--arc-border-subtle);
}

.v2-publisher-facts > div {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 9px 12px;
  border-right: 1px solid var(--arc-border-subtle);
}

.v2-publisher-facts > div:last-child {
  border-right: 0;
}

.v2-publisher-facts dt {
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.35;
}

.v2-publisher-facts dd {
  margin: 0;
  color: var(--arc-text-primary);
  font-size: 14px;
  font-weight: 800;
}

.v2-publisher-relay-scope,
.v2-publisher-summary-state {
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 10px;
  line-height: 1.5;
}

.v2-publisher-scope-warning {
  display: flex;
  gap: 8px;
  border-left: 2px solid var(--arc-warning);
  padding: 8px 10px;
  background: var(--arc-surface-recessed);
  color: var(--arc-text-secondary);
  font-size: 10.5px;
}

.v2-publisher-game-row {
  min-width: 0;
  display: grid;
  grid-template-columns: 44px minmax(0, 1fr) auto;
  align-items: center;
  gap: 16px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
}

.v2-publisher-row-art {
  width: 44px;
  height: 44px;
  overflow: hidden;
  border-radius: var(--arc-radius-sm);
}

.v2-publisher-row-art .arc-game-artwork {
  min-height: 0;
}

.v2-publisher-row-art .arc-artwork-fallback {
  padding: 3px;
  font-size: 6px;
  letter-spacing: 0;
}

.v2-publisher-row-copy {
  min-width: 0;
}

.v2-publisher-row-title {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.v2-publisher-row-title h2 {
  min-width: 0;
  margin: 0;
  overflow: hidden;
  font: 700 13px/1.35 var(--arc-font-mono);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.v2-publisher-row-title span,
.v2-publisher-row-id,
.v2-publisher-row-meta,
.v2-publisher-row-campaign {
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-row-statuses {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 6px;
}

.v2-publisher-row-campaign {
  margin-top: 5px;
}

.v2-publisher-row-meta {
  margin-top: 5px;
}

.v2-publisher-row-actions {
  position: relative;
  max-width: 150px;
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 5px;
}

.v2-publisher-row-more {
  position: relative;
}

.v2-publisher-row-more > summary {
  min-height: 32px;
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--arc-border-strong);
  border-radius: var(--arc-radius-xs);
  padding: 7px 10px;
  color: var(--arc-text-secondary);
  font-size: 11px;
  cursor: pointer;
  list-style: none;
}

.v2-publisher-row-more > summary::-webkit-details-marker {
  display: none;
}

.v2-publisher-row-more[open] > div {
  position: absolute;
  z-index: 20;
  top: calc(100% + 5px);
  right: 0;
  min-width: 190px;
  display: grid;
  gap: 4px;
  border: 1px solid var(--arc-border-strong);
  border-radius: var(--arc-radius-sm);
  padding: 6px;
  background: var(--arc-surface-lowest);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
}

.v2-publisher-row-more button {
  border: 0;
  border-radius: var(--arc-radius-xs);
  padding: 7px 9px;
  color: var(--arc-text-secondary);
  background: transparent;
  font: 600 10.5px/1.4 var(--arc-font-mono);
  text-align: left;
  cursor: pointer;
}

.v2-publisher-row-more button:hover,
.v2-publisher-row-more button:focus-visible {
  color: var(--arc-text-primary);
  background: var(--arc-surface-recessed);
}

.v2-publisher-attention,
.v2-publisher-campaigns,
.v2-publisher-unavailable-metrics {
  margin-top: 10px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
}

.v2-publisher-attention {
  border-color: oklch(0.5 0.13 300 / 50%);
  background: oklch(0.5 0.13 300 / 8%);
}

.v2-publisher-attention > p {
  margin: 0;
  color: var(--arc-info);
  font-size: 11px;
}

.v2-publisher-attention ul {
  display: grid;
  gap: 7px;
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
}

.v2-publisher-attention > ul > li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  border-top: 1px solid var(--arc-border-subtle);
  padding-top: 7px;
}

.v2-publisher-attention > ul > li > div {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.v2-publisher-attention > ul > li > div > strong,
.v2-publisher-attention > ul > li > div > span {
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-attention > ul > li > div > span {
  color: var(--arc-text-secondary);
}

.v2-publisher-attention li ul {
  display: grid;
  gap: 3px;
  margin: 3px 0 0;
  padding-left: 16px;
  color: var(--arc-text-secondary);
}

.v2-publisher-attention li li {
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-campaigns > summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  cursor: pointer;
  list-style: none;
}

.v2-publisher-campaigns > summary::-webkit-details-marker {
  display: none;
}

.v2-publisher-campaigns > summary > span:first-child {
  display: grid;
  gap: 3px;
}

.v2-publisher-campaigns > summary strong {
  font-size: 12px;
}

.v2-publisher-campaigns > summary > span:last-child {
  color: var(--arc-text-muted);
  font-size: 10.5px;
}

.v2-publisher-campaigns[open] > div {
  margin-top: 12px;
}

.v2-publisher-campaign-summary {
  display: grid;
  grid-template-columns: minmax(150px, 0.28fr) minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  border-top: 1px solid var(--arc-border-subtle);
  padding-top: 9px;
}

.v2-publisher-campaign-summary > div {
  min-width: 0;
  display: grid;
  gap: 3px;
}

.v2-publisher-campaign-summary > div span {
  overflow: hidden;
  color: var(--arc-text-muted);
  font-size: 10px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.v2-publisher-campaign-summary dl {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 8px;
  margin: 0;
}

.v2-publisher-campaign-summary dl div {
  min-width: 0;
}

.v2-publisher-campaign-summary dt,
.v2-publisher-campaign-summary dd {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: 10.5px;
  line-height: 1.4;
}

.v2-publisher-campaign-summary dt {
  color: var(--arc-text-muted);
}

.v2-publisher-summary-error {
  color: var(--arc-error);
}

.v2-publisher-unavailable-metrics {
  margin: 2px 0 0;
  border: 0;
  padding: 0 2px;
  background: transparent;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-publisher-release-summary {
  max-width: 760px;
}

.v2-publisher-release-summary .v2-publisher-dashboard-header {
  display: block;
}

.v2-publisher-release-summary .v2-publisher-dashboard-header h1 {
  margin-bottom: 6px;
}

.v2-publisher-release-summary .v2-publisher-dashboard-header p:last-child {
  max-width: 70ch;
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 11px;
  line-height: 1.55;
}

.v2-publisher-section-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
}

.v2-publisher-release-facts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  margin: 12px 0;
}

.v2-publisher-release-facts > div {
  min-width: 0;
  border-left: 2px solid var(--arc-border-strong);
  padding-left: 9px;
}

.v2-publisher-release-facts dt,
.v2-publisher-release-facts dd {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-release-facts dt {
  color: var(--arc-text-muted);
}

.v2-publisher-detail-facts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
  margin: 12px 0 0;
}

.v2-publisher-detail-facts > div {
  min-width: 0;
  border-left: 2px solid var(--arc-border-strong);
  padding-left: 9px;
}

.v2-publisher-detail-facts dt,
.v2-publisher-detail-facts dd {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-detail-facts dt {
  color: var(--arc-text-muted);
}

/* Manage Game */
.v2-publisher-manage {
  display: grid;
  gap: 14px;
}

.v2-publisher-manage-header {
  display: grid;
  grid-template-columns: 68px minmax(0, 1fr) auto;
  align-items: center;
  gap: 14px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
}

.v2-publisher-manage-art {
  width: 68px;
  height: 68px;
  overflow: hidden;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-sm);
}

.v2-publisher-manage-art .arc-game-artwork,
.v2-publisher-manage-art .arc-artwork-fallback {
  width: 100%;
  height: 100%;
}

.v2-publisher-manage-identity {
  min-width: 0;
  display: grid;
  gap: 5px;
}

.v2-publisher-manage-identity h1 {
  margin: 0;
  font-size: 16px;
  line-height: 1.3;
}

.v2-publisher-manage-coordinate,
.v2-publisher-manage-meta {
  margin: 0;
  overflow-wrap: anywhere;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-manage-coordinate {
  color: var(--arc-text-muted);
}

.v2-publisher-manage-cards {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  max-width: 760px;
}

.v2-publisher-manage-card {
  min-width: 0;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
}

.v2-publisher-manage-card h2 {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.35;
}

.v2-publisher-manage-card-state {
  margin: 4px 0 0;
  overflow-wrap: anywhere;
  color: var(--arc-text-primary);
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-manage-card-detail {
  margin: 4px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-publisher-manage {
  width: 100%;
  max-width: none;
  margin: 0;
}

.v2-publisher-manage h2,
.v2-publisher-manage h3 {
  margin-bottom: 0;
  font-family: var(--arc-font-mono);
  font-size: 12.5px;
  line-height: 1.35;
  letter-spacing: 0;
}

.v2-publisher-manage .v2-publisher-kicker {
  margin-bottom: 0;
  color: var(--arc-text-muted);
  font-size: 10px;
  letter-spacing: 1.5px;
}

.v2-publisher-manage .v2-btn-primary,
.v2-publisher-manage .v2-btn-secondary {
  min-height: 32px;
  border-radius: var(--arc-radius-xs);
  padding: 7px 12px;
  font-size: 11px;
}

.v2-publisher-manage .v2-publisher-panel {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
  box-shadow: none;
}

.v2-publisher-manage .v2-publisher-main > .v2-publisher-panel > * + * {
  margin-top: 9px;
}

.v2-publisher-manage .v2-publisher-management-layout,
.v2-publisher-manage .v2-publisher-main,
.v2-publisher-manage .v2-publisher-promotion-list {
  gap: 12px;
}

.v2-publisher-manage .v2-publisher-section-heading p {
  margin: 3px 0 0;
}

.v2-publisher-manage .v2-publisher-sidebar {
  top: 78px;
  gap: 9px;
}

.v2-publisher-manage .v2-publisher-sidebar h3 {
  margin: 0 0 3px;
  font-size: 11px;
}

@media (max-width: 980px) {
  .v2-publisher-facts {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .v2-publisher-detail-facts {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .v2-publisher-facts > div:nth-child(2) {
    border-right: 0;
  }

  .v2-publisher-facts > div:nth-child(n + 3) {
    border-top: 1px solid var(--arc-border-subtle);
  }

  .v2-publisher-campaign-summary dl {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (max-width: 760px) {
  .v2-publisher-dashboard-header,
  .v2-publisher-row-title,
  .v2-publisher-attention li {
    align-items: flex-start;
    flex-direction: column;
  }

  .v2-publisher-game-row {
    grid-template-columns: 44px minmax(0, 1fr);
  }

  .v2-publisher-row-actions {
    max-width: none;
    grid-column: 1 / -1;
    justify-content: flex-start;
  }

  .v2-publisher-manage-header {
    grid-template-columns: 68px minmax(0, 1fr);
  }

  .v2-publisher-manage-header .v2-publisher-actions {
    grid-column: 1 / -1;
  }

  .v2-publisher-manage-cards,
  .v2-publisher-detail-facts {
    grid-template-columns: minmax(0, 1fr);
  }

  .v2-publisher-campaign-summary {
    grid-template-columns: 1fr;
  }
}

/* Promotions and campaign management */
.v2-publisher-editor {
  width: 100%;
  max-width: none;
  margin: 0;
  gap: 12px;
}

.v2-publisher-editor h1 {
  margin: 0;
  font: 800 18px/1.3 var(--arc-font-mono);
  letter-spacing: 0;
}

.v2-publisher-editor h2,
.v2-publisher-editor h3 {
  margin: 0 0 8px;
  font-family: var(--arc-font-mono);
  font-size: 12.5px;
  line-height: 1.35;
  letter-spacing: 0;
}

.v2-publisher-editor h3 {
  font-size: 11px;
}

.v2-publisher-editor p,
.v2-publisher-editor li,
.v2-publisher-editor label,
.v2-publisher-editor legend,
.v2-publisher-editor dt,
.v2-publisher-editor dd,
.v2-publisher-editor summary {
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-publisher-editor .v2-publisher-kicker {
  margin-bottom: 3px;
  color: var(--arc-text-muted);
  font-size: 10px;
  letter-spacing: 1.5px;
}

.v2-publisher-editor .v2-publisher-panel,
.v2-publisher-editor .v2-publisher-game-hero {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
  box-shadow: none;
}

/* The shared hero uses space-between for its two-column dashboard use; with only
   artwork plus a title block that pushed the copy to the far right edge. */
.v2-publisher-editor .v2-publisher-game-hero {
  align-items: center;
  justify-content: flex-start;
  gap: 14px;
}

.v2-publisher-editor .v2-publisher-game-hero > div:last-child {
  min-width: 0;
  flex: 1;
}

.v2-publisher-editor .v2-btn-primary,
.v2-publisher-editor .v2-btn-secondary {
  min-height: 32px;
  border-radius: var(--arc-radius-xs);
  padding: 7px 12px;
  font-size: 11px;
}

.v2-publisher-editor .v2-publisher-management-layout,
.v2-publisher-editor .v2-publisher-main,
.v2-publisher-editor .v2-publisher-form {
  gap: 12px;
}

.v2-publisher-editor .v2-publisher-sidebar {
  top: 78px;
}

.v2-publisher-editor .v2-publisher-sidebar ul {
  gap: 5px;
  padding-left: 16px;
  color: var(--arc-text-secondary);
}

/* Campaign rows */
.v2-publisher-promotion-row {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 12px 14px;
  background: var(--arc-surface);
}

.v2-campaign-id {
  overflow-wrap: anywhere;
  font-size: 11px;
  font-weight: 700;
}

.v2-campaign-window,
.v2-campaign-mode,
.v2-campaign-help {
  margin: 6px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-campaign-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 10px;
}

.v2-campaign-note {
  margin: 8px 0 0;
  overflow-wrap: anywhere;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-campaign-note-ok {
  color: var(--arc-success);
}

.v2-campaign-blocker {
  margin: 8px 0 0;
  overflow-wrap: anywhere;
  color: var(--arc-error);
  font-size: 10.5px;
  line-height: 1.5;
}

/* Campaign publication lifecycle */
.v2-campaign-publication {
  margin-top: 10px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface-recessed);
}

.v2-campaign-overall {
  margin: 0 0 9px;
  font-size: 11px;
  font-weight: 700;
}

.v2-campaign-stage-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 9px;
  margin: 0;
}

.v2-campaign-stage-grid > div {
  min-width: 0;
  border-left: 2px solid var(--arc-border-strong);
  padding-left: 9px;
}

.v2-campaign-stage-grid dt {
  margin: 0 0 4px;
  color: var(--arc-text-muted);
}

.v2-campaign-stage-grid dd {
  margin: 0;
}

/* Confirmation dialog */
.v2-publisher-dialog-copy {
  display: grid;
  gap: 6px;
}

.v2-publisher-dialog-title {
  margin: 0;
  font-family: var(--arc-font-mono);
  font-size: 13px;
  line-height: 1.35;
}

.v2-publisher-dialog-message {
  margin: 0;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
  line-height: 1.55;
}

.v2-publisher-dialog-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 9px;
  margin-top: 14px;
}

@media (max-width: 760px) {
  .v2-campaign-stage-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

/* Create Game workflow */
.v2-create-workflow {
  width: 100%;
  display: grid;
  gap: 12px;
  max-width: none;
  margin: 0;
  padding-bottom: 12px;
}

.v2-create-header {
  display: grid;
  gap: 3px;
}

.v2-create-workflow .v2-create-title {
  margin: 0;
  font: 800 18px/1.3 var(--arc-font-mono);
  letter-spacing: 0;
}

.v2-create-workflow h2,
.v2-create-workflow h3 {
  margin: 0;
  font-family: var(--arc-font-mono);
  letter-spacing: 0;
}

.v2-create-workflow .v2-create-section-title {
  font-size: 13px;
  line-height: 1.35;
}

.v2-create-workflow .v2-create-subsection-title {
  font-size: 11.5px;
  line-height: 1.35;
}

.v2-create-workflow .v2-input,
.v2-create-workflow select,
.v2-create-workflow textarea {
  font-size: 11.5px;
  line-height: 1.5;
}

.v2-create-workflow textarea.v2-input {
  min-height: 90px;
  resize: vertical;
}

.v2-create-subtitle {
  margin: 0;
  max-width: 72ch;
  color: var(--arc-text-muted);
  font-size: 11.5px;
  line-height: 1.5;
}

.v2-create-stages ol {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
  margin: 0;
  padding: 0;
  list-style: none;
  max-width: 760px;
}

.v2-create-stage {
  width: 100%;
  min-width: 0;
  display: grid;
  gap: 2px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 8px 10px;
  background: var(--arc-surface);
  color: inherit;
  font-family: var(--arc-font-mono);
  text-align: left;
  cursor: pointer;
}

.v2-create-stage:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.v2-create-stage:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.v2-create-stage-index {
  color: var(--arc-text-muted);
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 1.2px;
  text-transform: uppercase;
}

.v2-create-stage-title {
  overflow-wrap: anywhere;
  font-size: 11.5px;
  font-weight: 700;
  line-height: 1.35;
}

.v2-create-stage-status {
  font-size: 10px;
  line-height: 1.4;
}

.v2-create-stage-complete {
  border-color: oklch(0.5 0.16 145 / 55%);
}

.v2-create-stage-complete .v2-create-stage-status {
  color: var(--arc-success);
}

.v2-create-stage-attention {
  border-color: oklch(0.6 0.18 25 / 55%);
}

.v2-create-stage-attention .v2-create-stage-status {
  color: var(--arc-error);
}

.v2-create-stage-current {
  border-color: var(--arc-accent);
  background: var(--arc-surface-recessed);
}

.v2-create-stage-current .v2-create-stage-status {
  color: var(--arc-accent);
}

.v2-create-stage-upcoming .v2-create-stage-status {
  color: var(--arc-text-muted);
}

.v2-create-persistence-note {
  margin: 0;
  max-width: 760px;
  border-left: 2px solid var(--arc-warning);
  padding-left: 9px;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-create-surface {
  display: grid;
  gap: 14px;
  width: 100%;
  max-width: 560px;
  margin-inline: auto;
}

.v2-create-stage-panel {
  min-width: 0;
  display: grid;
  gap: 12px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 16px;
  background: var(--arc-surface);
}

.v2-create-section-title {
  margin: 0;
  font-family: var(--arc-font-mono);
  font-size: 13px;
  line-height: 1.35;
}

.v2-create-subsection-title {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.35;
}

.v2-create-panel-heading {
  display: flex;
  align-items: center;
  gap: 9px;
}

.v2-create-kicker {
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 10px;
  font-weight: 800;
  letter-spacing: 1.5px;
  text-transform: uppercase;
}

.v2-create-fields {
  display: grid;
  gap: 14px;
  min-width: 0;
}

.v2-create-field {
  min-width: 0;
}

.v2-create-field-label {
  display: block;
  margin-bottom: 6px;
  color: var(--arc-text-muted);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.6px;
  text-transform: uppercase;
}

.v2-create-help {
  margin: 6px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-create-field-pair {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.v2-create-chip-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.v2-create-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-xs);
  padding: 4px 8px;
  background: var(--arc-surface-recessed);
  font-size: 10.5px;
}

.v2-create-chip-remove {
  border: 0;
  background: transparent;
  color: var(--arc-error);
  font-family: inherit;
  font-size: 10px;
  cursor: pointer;
}

.v2-create-alert {
  margin: 0;
  display: grid;
  gap: 5px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 10px 12px;
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-create-alert ul {
  margin: 0;
  padding-left: 16px;
  display: grid;
  gap: 3px;
}

.v2-create-alert-error {
  border-color: oklch(0.6 0.18 25 / 55%);
  background: oklch(0.6 0.18 25 / 10%);
  color: var(--arc-error);
}

.v2-create-issue-link {
  border: 0;
  padding: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  text-decoration: underline;
  cursor: pointer;
}

.v2-create-inline-error {
  margin: 8px 0 0;
  color: var(--arc-error);
  font-size: 10.5px;
}

.v2-create-inline-note {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 0 0;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
}

.v2-create-inline-icon {
  font-size: 14px;
}

.v2-create-inline-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
}

.v2-create-inline-row .v2-input {
  flex: 1 1 12rem;
  min-width: 0;
}

.v2-create-row-between {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.v2-create-checkbox-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10.5px;
  color: var(--arc-text-secondary);
}

.v2-create-checkbox {
  margin-right: 8px;
}

.v2-create-mono-value,
.v2-create-account {
  display: block;
  margin: 8px 0 0;
  overflow-wrap: anywhere;
  border-radius: var(--arc-radius-xs);
  padding: 7px 9px;
  background: var(--arc-surface-recessed);
  font-family: var(--arc-font-mono);
  font-size: 10.5px;
}

.v2-create-mono-inline {
  font-family: var(--arc-font-mono);
}

/* Media selection */
.v2-create-cover {
  position: relative;
  overflow: hidden;
  width: 100%;
  height: 150px;
  border: 1px dashed var(--arc-border-strong);
  border-radius: var(--arc-radius-sm);
  background: var(--arc-surface-recessed);
}

.v2-create-cover-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  color: inherit;
  font-family: inherit;
  text-align: center;
  cursor: pointer;
}

.v2-create-drop-icon {
  display: block;
  margin: 0 auto;
  color: var(--arc-text-muted);
  font-size: 20px;
}

.v2-create-drop-title {
  display: block;
  margin-top: 6px;
  font-size: 11.5px;
  font-weight: 700;
}

.v2-create-media-image {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.v2-create-cover-overlay,
.v2-create-shot-overlay {
  position: absolute;
  inset-inline: 0;
  bottom: 0;
  padding: 8px;
  background: linear-gradient(to top, oklch(0 0 0 / 88%), transparent);
}

.v2-create-cover-overlay {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 8px;
}

.v2-create-media-meta {
  min-width: 0;
}

.v2-create-media-name,
.v2-create-media-detail {
  margin: 0;
  overflow: hidden;
  color: oklch(0.97 0 0);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.v2-create-media-name {
  font-weight: 700;
}

.v2-create-media-detail {
  color: oklch(0.97 0 0 / 72%);
}

.v2-create-media-actions {
  display: flex;
  flex-shrink: 0;
  gap: 6px;
  margin-top: 4px;
}

.v2-create-media-button {
  border: 1px solid oklch(1 0 0 / 30%);
  border-radius: var(--arc-radius-xs);
  padding: 4px 8px;
  background: oklch(0 0 0 / 60%);
  color: oklch(0.97 0 0);
  font-family: inherit;
  font-size: 10px;
  font-weight: 700;
  cursor: pointer;
}

.v2-create-media-link {
  border: 0;
  padding: 0;
  background: transparent;
  color: oklch(0.97 0 0);
  font-family: inherit;
  font-size: 10px;
  font-weight: 700;
  cursor: pointer;
}

.v2-create-media-remove {
  color: var(--arc-error);
  border-color: oklch(0.6 0.18 25 / 60%);
}

.v2-create-shot-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
  margin-top: 8px;
}

.v2-create-shot {
  position: relative;
  overflow: hidden;
  aspect-ratio: 1 / 1;
  border: 1px dashed var(--arc-border-strong);
  border-radius: var(--arc-radius-xs);
  background: var(--arc-surface-recessed);
}

.v2-create-shot-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--arc-text-muted);
  cursor: pointer;
}

.v2-create-shot-empty:disabled {
  cursor: not-allowed;
  opacity: 0.4;
}

.v2-create-subpanel {
  min-width: 0;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface-recessed);
}

.v2-create-subpanel-row {
  display: flex;
  align-items: flex-start;
  gap: 9px;
}

.v2-create-subpanel-icon {
  color: var(--arc-info);
  font-size: 16px;
}

.v2-create-subpanel-body {
  min-width: 0;
  flex: 1;
}

/* Builds and distribution */
.v2-create-toggle {
  display: flex;
  align-items: center;
  gap: 9px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 10px 12px;
  background: var(--arc-surface-recessed);
}

.v2-create-toggle-label {
  font-size: 11.5px;
  font-weight: 700;
}

.v2-create-disclosure {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 10px 12px;
  background: var(--arc-surface-recessed);
}

.v2-create-disclosure-summary {
  cursor: pointer;
  font-size: 11px;
  font-weight: 700;
}

.v2-create-mode-option {
  width: 100%;
  display: block;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface);
  color: inherit;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
}

.v2-create-mode-selected {
  border-color: var(--arc-accent);
  background: var(--arc-surface-recessed);
}

.v2-create-mode-title {
  font-size: 11.5px;
  font-weight: 700;
}

.v2-create-mode-title-active {
  color: var(--arc-accent);
}

.v2-create-selected-badge {
  border-radius: var(--arc-radius-xs);
  padding: 3px 8px;
  background: var(--arc-accent);
  color: var(--arc-background);
  font-size: 9.5px;
  font-weight: 800;
  letter-spacing: 1px;
  text-transform: uppercase;
}

.v2-create-selected-badge-idle {
  background: var(--arc-surface-recessed);
  color: var(--arc-text-muted);
}

.v2-create-list {
  display: grid;
  gap: 6px;
}

.v2-create-server-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-xs);
  padding: 8px 10px;
  background: var(--arc-surface);
  font-size: 10.5px;
}

.v2-create-server-row p {
  margin: 0;
  overflow-wrap: anywhere;
}

.v2-create-server-name {
  font-weight: 700;
}

.v2-create-server-status {
  text-align: right;
}

.v2-create-file-button {
  width: 100%;
  overflow-wrap: anywhere;
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-sm);
  padding: 9px 10px;
  background: var(--arc-surface-recessed);
  color: inherit;
  font-family: var(--arc-font-mono);
  font-size: 11px;
  text-align: left;
  cursor: pointer;
}

/* Review and publication */
.v2-create-summary-card {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface-recessed);
}

.v2-create-summary-title {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: 14px;
  font-weight: 800;
}

.v2-create-summary-meta {
  margin: 4px 0 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
}

.v2-create-review-facts {
  display: grid;
  gap: 8px;
  margin: 0;
}

.v2-create-review-facts > div {
  min-width: 0;
  border-left: 2px solid var(--arc-border-strong);
  padding-left: 9px;
}

.v2-create-review-facts dt,
.v2-create-review-facts dd {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-create-review-facts dt {
  color: var(--arc-text-muted);
}

.v2-create-review-prewrap {
  white-space: pre-wrap;
}

.v2-create-authorization {
  border-top: 1px solid var(--arc-border-subtle);
  padding-top: 10px;
}

.v2-create-checklist,
.v2-create-warnings {
  display: grid;
  gap: 5px;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-create-checklist li::before {
  margin-right: 6px;
  content: "\2022";
}

.v2-create-check-ok {
  color: var(--arc-success);
}

.v2-create-check-ok::before {
  content: "\2713" !important;
}

.v2-create-check-blocked {
  color: var(--arc-error);
}

.v2-create-check-blocked::before {
  content: "\2715" !important;
}

.v2-create-warnings {
  color: var(--arc-warning);
}

.v2-create-status-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.v2-create-phase {
  border: 1px solid var(--arc-border-control);
  border-radius: var(--arc-radius-xs);
  padding: 3px 8px;
  font-size: 10px;
  font-weight: 700;
  white-space: nowrap;
}

.v2-create-phase-idle {
  color: var(--arc-text-muted);
}

.v2-create-phase-busy {
  border-color: oklch(0.5 0.13 195 / 60%);
  color: var(--arc-info);
}

.v2-create-phase-warning {
  border-color: oklch(0.72 0.15 75 / 60%);
  color: var(--arc-warning);
}

.v2-create-phase-ok {
  border-color: oklch(0.5 0.16 145 / 60%);
  color: var(--arc-success);
}

.v2-create-phase-error {
  border-color: oklch(0.6 0.18 25 / 60%);
  color: var(--arc-error);
}

.v2-create-status-ok,
.v2-create-status-error,
.v2-create-status-busy,
.v2-create-status-neutral {
  margin: 8px 0 0;
  overflow-wrap: anywhere;
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-create-status-ok {
  color: var(--arc-success);
}

.v2-create-status-error {
  color: var(--arc-error);
}

.v2-create-status-busy {
  color: var(--arc-info);
}

.v2-create-status-neutral {
  color: var(--arc-text-secondary);
}

.v2-create-progress {
  margin-top: 10px;
}

.v2-create-progress-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 5px;
  font-size: 10px;
  font-weight: 700;
}

.v2-create-progress-track {
  overflow: hidden;
  height: 6px;
  border-radius: 999px;
  background: var(--arc-progress-track);
}

.v2-create-progress-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--arc-accent);
}

.v2-create-progress-log {
  display: grid;
  gap: 4px;
  margin: 10px 0 0;
  padding-left: 16px;
  color: var(--arc-text-muted);
  font-size: 10px;
  line-height: 1.45;
}

/* Action area */
.v2-create-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  max-width: 560px;
  margin-inline: auto;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 10px 12px;
  background: var(--arc-surface);
}

.v2-create-actions-state {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.v2-create-authoring {
  font-size: 10.5px;
  font-weight: 700;
}

.v2-create-authoring-dirty {
  color: var(--arc-text-muted);
  font-size: 10px;
}

.v2-create-actions-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.v2-create-workflow .v2-btn-primary,
.v2-create-workflow .v2-btn-secondary {
  min-height: 32px;
  border-radius: var(--arc-radius-xs);
  padding: 7px 14px;
  font-size: 11px;
}

@media (max-width: 760px) {
  .v2-create-stages ol {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .v2-create-field-pair,
  .v2-create-shot-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .v2-create-actions {
    align-items: flex-start;
    flex-direction: column;
  }
}

/* Typed Store Page editor */
.v2-store-page-editor {
  min-width: 0;
  padding-bottom: 6.5rem;
  overflow-x: clip;
}

/* Phase 9: retune the editor onto the canonical token system without disturbing
   the shared publisher chrome used by other surfaces. */
.v2-store-page-editor {
  width: 100%;
  max-width: none;
  margin: 0;
  gap: 12px;
}

.v2-store-page-editor h1 {
  margin: 0;
  font: 800 18px/1.3 var(--arc-font-mono);
  letter-spacing: 0;
}

.v2-store-page-editor h2,
.v2-store-page-editor h3 {
  margin: 0 0 8px;
  font-family: var(--arc-font-mono);
  font-size: 12.5px;
  line-height: 1.35;
  letter-spacing: 0;
}

.v2-store-page-editor h3 {
  font-size: 11px;
}

.v2-store-page-editor .v2-publisher-panel {
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 14px 16px;
  background: var(--arc-surface);
  box-shadow: none;
}

.v2-store-page-editor .v2-publisher-game-hero {
  align-items: center;
  padding: 14px 16px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
  box-shadow: none;
}

.v2-store-page-editor .v2-publisher-kicker {
  margin-bottom: 3px;
  color: var(--arc-text-muted);
  font-size: 10px;
  letter-spacing: 1.5px;
}

.v2-store-page-editor .v2-btn-primary,
.v2-store-page-editor .v2-btn-secondary {
  min-height: 32px;
  border-radius: var(--arc-radius-xs);
  padding: 7px 12px;
  font-size: 11px;
}

.v2-store-page-editor .v2-store-editor-tabs {
  top: 72px;
  gap: 5px;
  padding: 6px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
}

.v2-store-page-editor .v2-store-editor-tabs button {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  padding: 6px 10px;
  border-radius: var(--arc-radius-xs);
  color: var(--arc-text-muted);
  font-family: var(--arc-font-mono);
  font-size: 11px;
}

.v2-store-page-editor .v2-store-editor-tabs button:hover,
.v2-store-page-editor .v2-store-editor-tab-active {
  color: var(--arc-text-primary) !important;
  border-color: var(--arc-border-control) !important;
  background: var(--arc-surface-recessed) !important;
}

.v2-store-page-editor .v2-store-editor-tab-active {
  border-color: var(--arc-accent) !important;
}

.v2-store-tab-flag {
  font-size: 10px;
  font-weight: 800;
  line-height: 1;
}

/* Editor body copy: the legacy block left lists, labels, and help text on the
   old display scale, which dwarfed the migrated panels. */
.v2-store-page-editor p,
.v2-store-page-editor li,
.v2-store-page-editor label,
.v2-store-page-editor small,
.v2-store-page-editor legend,
.v2-store-page-editor strong,
.v2-store-page-editor summary,
.v2-store-page-editor dt,
.v2-store-page-editor dd {
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-store-page-editor label {
  display: grid;
  gap: 5px;
}

.v2-store-page-editor ul {
  display: grid;
  gap: 4px;
  margin: 0 0 10px;
  padding-left: 16px;
}

.v2-store-page-editor .v2-store-outcome-list {
  padding-left: 0;
}

.v2-store-page-editor .v2-input {
  font-size: 11.5px;
  line-height: 1.5;
}

.v2-store-page-editor .v2-store-readiness h3 {
  margin: 10px 0 5px;
}

.v2-store-page-editor .v2-store-canonical-placeholder {
  border-radius: var(--arc-radius-sm);
  padding: 10px 12px;
  text-align: left;
}

.v2-store-page-editor .v2-store-canonical-placeholder strong {
  font-size: 11px;
}

/* The floating footer overlapped the readiness column; dock it in flow instead. */
.v2-store-page-editor {
  padding-bottom: 12px;
}

.v2-store-page-editor .v2-store-editor-footer {
  position: sticky;
  right: auto;
  bottom: 8px;
  z-index: 20;
  flex-wrap: wrap;
  gap: 8px;
  max-width: none;
  padding: 10px 12px;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  background: var(--arc-surface);
  box-shadow: none;
}

.v2-store-tab-flag-blocked {
  color: var(--arc-error);
}

.v2-store-tab-flag-warned {
  color: var(--arc-warning);
}

.v2-store-fieldset {
  display: contents;
  border: 0;
  margin: 0;
  padding: 0;
}

.v2-store-editor-main {
  display: grid;
  gap: 12px;
}

.v2-store-page-editor .v2-store-card {
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface-recessed);
}

.v2-store-help,
.v2-store-persistence-note {
  margin: 0;
  color: var(--arc-text-muted);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-store-alert {
  margin: 0;
  color: var(--arc-error);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-store-ok {
  margin: 0;
  color: var(--arc-success);
  font-size: 10.5px;
}

.v2-store-notice {
  margin: 0;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 9px 11px;
  background: var(--arc-surface-recessed);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-store-span-all {
  grid-column: 1 / -1;
}

.v2-store-textarea-lg {
  min-height: 200px;
}

.v2-store-textarea-md {
  min-height: 110px;
}

/* Publication lifecycle: Store Page event and listing pointer stay separate. */
.v2-store-publication {
  border-color: oklch(0.72 0.15 75 / 55%) !important;
}

.v2-store-overall-status {
  margin: 0 0 9px;
  font-size: 11.5px;
  font-weight: 700;
}

.v2-store-stage-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 9px;
  margin: 0 0 10px;
}

.v2-store-stage-grid > div {
  min-width: 0;
  border-left: 2px solid var(--arc-border-strong);
  padding-left: 9px;
}

.v2-store-stage-grid dt,
.v2-store-stage-grid dd {
  margin: 0;
  font-size: 10.5px;
  line-height: 1.45;
}

.v2-store-stage-grid dt {
  color: var(--arc-text-muted);
}

.v2-store-stage-idle {
  color: var(--arc-text-muted);
}

.v2-store-stage-busy {
  color: var(--arc-info);
}

.v2-store-stage-warning {
  color: var(--arc-warning);
}

.v2-store-stage-ok {
  color: var(--arc-success);
}

.v2-store-stage-error {
  color: var(--arc-error);
}

.v2-store-outcome-list {
  display: grid;
  gap: 6px;
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
}

.v2-store-outcome-row {
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-xs);
  padding: 8px 10px;
  background: var(--arc-surface-recessed);
  font-size: 10.5px;
}

.v2-store-outcome-row p {
  margin: 3px 0 0;
  overflow-wrap: anywhere;
}

.v2-store-persistence {
  margin: 0;
  font-size: 11px;
  font-weight: 700;
}

.v2-store-revision {
  margin: 3px 0 0;
  overflow-wrap: anywhere;
  color: var(--arc-text-secondary);
  font-size: 10.5px;
}

.v2-store-persistence-note {
  margin-top: 6px;
  border-left: 2px solid var(--arc-warning);
  padding-left: 8px;
}

.v2-store-footer-status {
  font-size: 10.5px;
  font-weight: 700;
}

.v2-store-preview-commerce {
  margin: 12px 0;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-sm);
  padding: 10px 12px;
  background: var(--arc-surface-recessed);
  font-size: 10.5px;
}

.v2-store-dialog {
  margin: auto;
  border: 1px solid var(--arc-border-card);
  border-radius: var(--arc-radius-md);
  padding: 18px;
  max-width: min(420px, 92vw);
  background: var(--arc-surface);
  color: var(--arc-text-primary);
  font-family: var(--arc-font-mono);
}

.v2-store-dialog-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 9px;
  margin-top: 14px;
}

.v2-blossom-status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}

.v2-blossom-phase {
  color: var(--arc-text-secondary);
  font-size: 10.5px;
}

.v2-blossom-dialog {
  max-width: min(46rem, 94vw);
  max-height: 90vh;
  overflow: auto;
}

/* Buyer-facing Store Page detail rendering, shared by the editor preview. */
.v2-detail-prose {
  color: var(--arc-text-secondary);
  font-size: 11.5px;
  line-height: 1.6;
}

.v2-detail-grid {
  display: grid;
  gap: 7px;
  margin-top: 10px;
}

.v2-detail-note {
  margin-top: 10px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-sm);
  padding: 9px 11px;
  background: var(--arc-surface-recessed);
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-detail-requirement {
  margin-top: 12px;
  border: 1px solid var(--arc-border-subtle);
  border-radius: var(--arc-radius-sm);
  padding: 12px;
  background: var(--arc-surface-recessed);
}

.v2-detail-requirement-grid {
  display: grid;
  gap: 4px;
  font-size: 10.5px;
  line-height: 1.5;
}

.v2-detail-link-row {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 10px;
}

.v2-detail-link {
  border: 1px solid var(--arc-border-control);
  border-radius: 999px;
  padding: 6px 12px;
  background: var(--arc-surface-recessed);
  color: var(--arc-accent);
  font-size: 10.5px;
  text-decoration: none;
}

.v2-detail-link:focus-visible {
  outline: 2px solid var(--arc-focus-ring);
  outline-offset: 2px;
}

.v2-detail-link-disabled {
  color: var(--arc-text-muted);
}

@media (min-width: 640px) {
  .v2-detail-requirement-grid {
    grid-template-columns: 10rem minmax(0, 1fr);
  }
}

@media (max-width: 980px) {
  .v2-store-stage-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 760px) {
  .v2-store-page-editor .v2-store-form-grid {
    grid-template-columns: minmax(0, 1fr);
  }
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

@media (max-width: 820px) {
  .v2-detail-layout {
    grid-template-columns: 1fr;
  }

  .v2-detail-sidebar {
    position: static;
    grid-row: 1;
    max-height: none;
    overflow: visible;
  }

  .v2-detail-main-column {
    grid-row: 2;
  }
}

@media (max-width: 720px) {
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
  .v2-detail-layout,
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

  .v2-detail-media {
    grid-auto-columns: minmax(250px, 82%);
  }

  .v2-detail-sidebar {
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
    bottom: 0.5rem;
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

@media (max-width: 520px) {
  .v2-detail-hero {
    height: 260px;
  }

  .v2-detail-title {
    left: 18px;
    right: 18px;
    bottom: 16px;
  }

  .v2-detail-buy-panel,
  .v2-detail-ownership-panel {
    padding: 18px;
  }
}

@media (min-width: 721px) {
  .v2-hide-desktop {
    display: none !important;
  }
}

@media (prefers-reduced-motion: reduce) {
  .v2-install-progress-indeterminate::after {
    width: 100%;
    animation: none !important;
    opacity: 0.65;
    transform: none;
  }

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

    #[test]
    fn detail_layout_tracks_the_handoff_geometry() {
        assert!(UI_V2_STYLES.contains(
            ".v2-detail-layout {\n  display: grid;\n  grid-template-columns: minmax(0, 1fr) 380px;\n  gap: 26px;"
        ));
        assert!(UI_V2_STYLES.contains(".v2-detail-hero {\n  position: relative;\n  height: 340px;"));
        assert!(UI_V2_STYLES.contains(".v2-detail-layout,\n  .v2-detail-grid,"));
    }
}
