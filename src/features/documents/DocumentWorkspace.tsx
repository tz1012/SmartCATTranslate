import { useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelDocumentTranslation, chooseDocument, openDocumentResult, translateDocument } from './documentApi';
import type { ChosenDocument, DocumentOptions, DocumentProgress, DocumentReport } from './types';

const defaults: DocumentOptions = { includeComments: true, includeNotes: true, includeHidden: false, wrapText: true, targetLanguage: 'ko' };
export function DocumentWorkspace({ locale }: { locale: 'ko' | 'en' }) {
  const ko = locale === 'ko'; const [options, setOptions] = useState(defaults); const [chosen, setChosen] = useState<ChosenDocument>();
  const [busy, setBusy] = useState(false); const [jobId, setJobId] = useState<string>(); const [progress, setProgress] = useState<DocumentProgress>(); const [report, setReport] = useState<DocumentReport>(); const [error, setError] = useState<string>();
  useEffect(() => { let disposed = false; let stop: undefined | (() => void); void listen<DocumentProgress>('document-progress', event => { if (!jobId || event.payload.jobId === jobId) setProgress(event.payload); }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }); return () => { disposed = true; stop?.(); }; }, [jobId]);
  const percent = useMemo(() => progress?.stage === 'inspect' ? 5 : progress?.stage === 'validate' ? 92 : progress ? 10 + Math.round(75 * progress.completed / Math.max(1, progress.total)) : 0, [progress]);
  const patch = (key: keyof DocumentOptions, value: boolean | string) => setOptions(current => ({ ...current, [key]: value }));
  const choose = async () => { setError(undefined); setReport(undefined); try { const value = await chooseDocument(options); if (value) setChosen(value); } catch (reason) { setError(String(reason)); } };
  const start = async () => { if (!chosen) return; const id = crypto.randomUUID(); setJobId(id); setBusy(true); setError(undefined); setReport(undefined); try { setReport(await translateDocument(id, chosen.sourcePath, options)); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const cancel = async () => { if (jobId) await cancelDocumentTranslation(jobId); };
  return <section className="document-workspace" aria-labelledby="document-title">
    <h2 id="document-title">{ko ? '서식 유지 문서 번역' : 'Format-preserving document translation'}</h2>
    <p>{ko ? 'DOCX, PPTX, XLSX의 텍스트만 번역해 원본 옆에 새 파일로 저장합니다.' : 'Translate only text in DOCX, PPTX and XLSX and save a new file beside the original.'}</p>
    <div className="document-picker"><button type="button" className="primary-action" onClick={() => void choose()} disabled={busy}>{ko ? '문서 선택' : 'Choose document'}</button>{chosen && <div role="status"><strong>{chosen.manifest.fileName}</strong><span>{chosen.manifest.format.toUpperCase()} · {chosen.manifest.segmentCount} {ko ? '개 텍스트 영역' : 'text segments'}</span></div>}</div>
    <fieldset disabled={busy}><legend>{ko ? '번역 옵션' : 'Translation options'}</legend>
      <label>{ko ? '대상 언어' : 'Target language'}<select value={options.targetLanguage} onChange={event => patch('targetLanguage', event.target.value)}><option value="ko">한국어</option><option value="en">English</option><option value="ja">日本語</option><option value="zh">中文</option><option value="de">Deutsch</option><option value="fr">Français</option><option value="es">Español</option></select></label>
      <label><input type="checkbox" checked={options.includeComments} onChange={event => patch('includeComments', event.target.checked)} />{ko ? '댓글·메모 포함' : 'Include comments'}</label>
      <label><input type="checkbox" checked={options.includeNotes} onChange={event => patch('includeNotes', event.target.checked)} />{ko ? '발표자 노트 포함' : 'Include speaker notes'}</label>
      <label><input type="checkbox" checked={options.wrapText} onChange={event => patch('wrapText', event.target.checked)} />{ko ? '스프레드시트 줄 바꿈 권장' : 'Prefer spreadsheet wrapping'}</label>
    </fieldset>
    <div className="document-actions"><button type="button" className="primary-action" disabled={!chosen || busy} onClick={() => void start()}>{ko ? '새 번역 파일 만들기' : 'Create translated copy'}</button>{busy && <button type="button" onClick={() => void cancel()}>{ko ? '취소' : 'Cancel'}</button>}</div>
    {busy && <div className="document-progress" role="status"><progress max="100" value={percent} /><span>{ko ? '문서를 처리하고 있습니다…' : 'Processing document…'} {percent}%</span></div>}
    {error && <p role="alert">{ko ? `문서 번역 실패: ${error}` : `Document translation failed: ${error}`}</p>}
    {report && <article className="document-report"><h3>{ko ? '번역 파일 완성' : 'Translated document ready'}</h3><p><strong>{report.outputName}</strong> · {report.translatedSegments} {ko ? '개 영역 번역' : 'segments translated'}</p>{report.warnings.length > 0 && <details><summary>{ko ? `레이아웃 확인 권장 ${report.warnings.length}건` : `${report.warnings.length} layout warnings`}</summary><ul>{report.warnings.slice(0, 100).map((warning, index) => <li key={`${warning.location}-${index}`}>{warning.location}: {warning.message}</li>)}</ul></details>}<button type="button" onClick={() => void openDocumentResult(report.outputPath)}>{ko ? '결과 파일 열기' : 'Open result'}</button></article>}
  </section>;
}
