export function App() {
  return (
    <main>
      <header>
        <h1>SmartCAT Translate</h1>
      </header>
      <section className="translation-grid" aria-label="텍스트 번역">
        <label>
          원문
          <textarea aria-label="원문" />
        </label>
        <label>
          번역문
          <textarea aria-label="번역문" readOnly />
        </label>
      </section>
    </main>
  );
}
