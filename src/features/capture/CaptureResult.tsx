import { useMemo, useState } from 'react';
import { exportTranslatedImage, updateCaptureBlock } from './captureApi';
import type { CaptureJobResult, TranslatedBlock } from './types';

type ViewMode = 'translated' | 'original' | 'sideBySide';

export function CaptureResult({ result, locale, onChange, onRetry }: { result: CaptureJobResult; locale: 'ko' | 'en'; onChange: (value: CaptureJobResult) => void; onRetry: () => void }) {
  const ko = locale === 'ko';
  const [view, setView] = useState<ViewMode>('sideBySide');
  const [saving, setSaving] = useState(false);
  const sourceText = useMemo(() => result.translatedBlocks.map((block) => block.sourceText).join('\n\n'), [result]);
  const translatedText = useMemo(() => result.translatedBlocks.filter((block) => block.visible).map((block) => block.translatedText).join('\n\n'), [result]);
  const copy = (text: string) => navigator.clipboard.writeText(text);
  const update = async (block: TranslatedBlock, translatedText: string, visible = block.visible) => onChange(await updateCaptureBlock(result.jobId, block.id, translatedText, visible));
  return <section className="capture-result" aria-labelledby="capture-result-title">
    <header><div><h3 id="capture-result-title">{ko ? '번역 결과' : 'Translation result'}</h3><span>{result.imageWidth} × {result.imageHeight}</span></div><div className="capture-view-tabs" role="group" aria-label={ko ? '미리보기 방식' : 'Preview mode'}>
      <button type="button" aria-pressed={view === 'original'} onClick={() => setView('original')}>{ko ? '원본' : 'Original'}</button>
      <button type="button" aria-pressed={view === 'translated'} onClick={() => setView('translated')}>{ko ? '번역' : 'Translated'}</button>
      <button type="button" aria-pressed={view === 'sideBySide'} onClick={() => setView('sideBySide')}>{ko ? '나란히' : 'Side by side'}</button>
    </div></header>
    <div className={`capture-preview capture-preview-${view}`}>
      {view !== 'translated' && result.sourcePreview && <figure><img src={result.sourcePreview} alt={ko ? '원본 이미지' : 'Original image'} /><figcaption>{ko ? '원본' : 'Original'}</figcaption></figure>}
      {view !== 'original' && result.translatedPreview && <figure><img src={result.translatedPreview} alt={ko ? '번역된 이미지' : 'Translated image'} /><figcaption>{ko ? '번역본' : 'Translated'}</figcaption></figure>}
    </div>
    {result.warnings.length > 0 && <aside className="capture-warnings" aria-label={ko ? '검토 필요' : 'Review needed'}><strong>{ko ? '검토가 필요한 영역' : 'Areas to review'}</strong><ul>{result.warnings.map((warning) => <li key={warning}>{warning.startsWith('textOverflow') ? (ko ? '글자가 영역보다 깁니다.' : 'Text exceeds its region.') : (ko ? '복잡한 배경을 근사 복원했습니다.' : 'Complex background was approximated.')}</li>)}</ul></aside>}
    <div className="capture-blocks">{result.translatedBlocks.map((block, index) => <article key={block.id} className={block.confidence < .65 ? 'low-confidence' : ''}>
      <header><strong>{ko ? `영역 ${index + 1}` : `Block ${index + 1}`}</strong><span>{Math.round(block.confidence * 100)}%</span></header>
      <p>{block.sourceText}</p><label>{ko ? '번역 수정' : 'Edit translation'}<textarea value={block.translatedText} onChange={(event) => onChange({ ...result, translatedBlocks: result.translatedBlocks.map((item) => item.id === block.id ? { ...item, translatedText: event.target.value } : item) })} onBlur={(event) => void update(block, event.target.value)} /></label>
      <div><label className="capture-visibility"><input type="checkbox" checked={block.visible} onChange={(event) => void update(block, block.translatedText, event.target.checked)} />{ko ? '이미지에 표시' : 'Show on image'}</label><button type="button" onClick={onRetry}>{ko ? '다시 번역' : 'Retry'}</button></div>
    </article>)}</div>
    <footer className="capture-result-actions"><button type="button" onClick={() => void copy(sourceText)}>{ko ? '원문 복사' : 'Copy source'}</button><button type="button" onClick={() => void copy(translatedText)}>{ko ? '번역문 복사' : 'Copy translation'}</button><select aria-label={ko ? '저장 형식' : 'Save format'} id="capture-export-format" defaultValue="png"><option value="png">PNG</option><option value="jpeg">JPEG</option><option value="webp">WebP</option></select><button className="primary-action" type="button" disabled={saving} onClick={async () => { setSaving(true); try { const element = document.getElementById('capture-export-format') as HTMLSelectElement; await exportTranslatedImage(result.jobId, element.value as 'png' | 'jpeg' | 'webp'); } finally { setSaving(false); } }}>{ko ? '새 이미지 저장' : 'Save new image'}</button></footer>
  </section>;
}
