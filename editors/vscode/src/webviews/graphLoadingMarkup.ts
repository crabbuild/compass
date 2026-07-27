export function graphStaticLoadingMarkup(): string {
  return `<main class="compass-load-shell compass-load-shell-static">
  <div class="compass-load-visual" aria-hidden="true">
    <span class="compass-load-mark">
      <svg class="compass-load-logo" viewBox="0 0 24 24" fill="none">
        <path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd"
          d="M3.554 21.529c1.797 1.221 4.943-.038 11.236-2.554 1.342-.537 2.013-.806 2.54-1.267q.201-.177.378-.378c.461-.527.73-1.198 1.267-2.54 2.515-6.293 3.775-9.44 2.554-11.236a4.1 4.1 0 0 0-1.083-1.083c-1.797-1.221-4.944.037-11.236 2.554-1.342.537-2.013.806-2.54 1.267q-.201.177-.378.378c-.461.527-.73 1.198-1.267 2.54-2.517 6.292-3.775 9.439-2.554 11.236.29.426.657.793 1.083 1.083M8.25 12a3.75 3.75 0 1 1 7.5 0 3.75 3.75 0 0 1-7.5 0m1.5 0a2.25 2.25 0 1 1 4.5 0 2.25 2.25 0 0 1-4.5 0"></path>
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
