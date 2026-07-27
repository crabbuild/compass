export function graphStaticLoadingMarkup(): string {
  return `<main class="compass-load-shell compass-load-shell-static">
  <div class="compass-load-constellation" aria-hidden="true">
    <span class="compass-load-mark">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"></circle>
        <polygon points="16 8 14 14 8 16 10 10 16 8"></polygon>
      </svg>
    </span>
    <span class="compass-load-progress"><i></i></span>
  </div>
  <section class="compass-load-copy" role="status" aria-live="polite">
    <span class="compass-load-eyebrow">Compass graph</span>
    <h1>Mapping your codebase</h1>
    <p class="compass-load-steps">
      <span class="compass-load-step" data-state="active"><i aria-hidden="true"></i>Reading graph</span>
      <span class="compass-load-step" data-state="pending"><i aria-hidden="true"></i>Arranging relationships</span>
      <span class="compass-load-step" data-state="pending"><i aria-hidden="true"></i>Preparing inspector</span>
    </p>
  </section>
</main>`;
}
