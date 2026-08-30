import { useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelDocumentTranslation, chooseDocument, translateDocument } from './documentApi';
import { DocumentOptions } from './DocumentOptions';
import { DocumentReport } from './DocumentReport';
import type { ChosenDocument, DocumentOptions as Options, DocumentProgress, DocumentReport as Report } from './types';

const defaults: Options = { includeComments: true, includeNotes: true, includeHidden: false, wrapText: true, targetLanguage: 'ko', sourceLanguage: null, profileId: null, model: null, pdfForceOcr: false, pdfFit: true, preserveAnnotations: true, outputDirectory: null };
const stages: Record<string, { ko: string; en: string }> = { inspect: { ko: '문서 검사', en: 'Inspecting' }, translate: { ko: '안전하게 번역', en: 'Translating safely' }, validate: { ko: '결과 다시 열어 검증', en: 'Validating output' } };

export function DocumentWorkspace({ locale }: { locale: 'ko' | 'en' }) {
  const ko = locale === 'ko'; const [options, setOptions] = useState(defaults); const [chosen, setChosen] = useState<ChosenDocument>();
  const [busy, setBusy] = useState(false); const [jobId, setJobId] = useState<string>(); const [progress, setProgress] = useState<DocumentProgress>(); const [report, setReport] = useState<Report>(); const [error, setError] = useState<string>();
  useEffect(() => { let disposed = false; let stop: undefined | (() => void); void listen<DocumentProgress>('document-progress', (event) => { if (!jobId || event.payload.jobId === jobId) setProgress(event.payload); }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; }); return () => { disposed = true; stop?.(); }; }, [jobId]);
  const percent = useMemo(() => progress?.stage === 'inspect' ? 5 : progress?.stage === 'validate' ? 94 : progress ? 12 + Math.round(74 * progress.completed / Math.max(1, progress.total)) : 0, [progress]);
  const choose = async () => { setError(undefined); setReport(undefined); try { const value = await chooseDocument(options); if (value) setChosen(value); } catch (reason) { setError(String(reason)); } };
  const start = async () => { if (!chosen) return; const id = crypto.randomUUID(); setJobId(id); setBusy(true); setProgress({ jobId: id, stage: 'inspect', completed: 0, total: 1 }); setError(undefined); setReport(undefined); try { setReport(await translateDocument(id, chosen.sourcePath, options)); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const cancel = async () => { if (jobId) await cancelDocumentTranslation(jobId); };
  const formatSummary = chosen?.manifest.format === 'pdf' ? `${chosen.manifest.pageCount} ${ko ? '페이지' : 'pages'} · ${chosen.manifest.pageKinds.filter((v) => v === 'scanned').length} OCR` : `${chosen?.manifest.segmentCount ?? 0} ${ko ? '개 텍스트 영역' : 'text segments'}`;
  return <section className="document-workspace" aria-labelledby="document-title">
    <h2 id="document-title">{ko ? '서식 유지 문서 번역' : 'Format-preserving document translation'}</h2>
    <p>{ko ? 'DOCX, PPTX, XLSX, PDF를 검사하고 원본을 건드리지 않은 새 번역 파일을 만듭니다.' : 'Inspect DOCX, PPTX, XLSX and PDF, then create a translated copy without changing the original.'}</p>
    <div className="document-picker"><button type="button" className="primary-action" onClick={() => void choose()} disabled={busy}>{ko ? '문서 선택' : 'Choose document'}</button>{chosen && <div role="status"><strong>{chosen.manifest.fileName}</strong><span className="format-badge">{chosen.manifest.format.toUpperCase()}</span><span>{formatSummary}</span>{chosen.manifest.hasSignatures && <span className="document-risk">{ko ? '전자서명 있음' : 'Contains signatures'}</span>}</div>}</div>
    <DocumentOptions locale={locale} value={options} format={chosen?.manifest.format} disabled={busy} onChange={setOptions}/>
    <div className="document-actions"><button type="button" className="primary-action" disabled={!chosen || busy} onClick={() => void start()}>{ko ? '새 번역 파일 만들기' : 'Create translated copy'}</button>{busy && <button type="button" onClick={() => void cancel()}>{ko ? '취소' : 'Cancel'}</button>}</div>
    {busy && <div className="document-progress" role="status" aria-live="polite"><progress max="100" value={percent}/><span>{stages[progress?.stage ?? 'inspect']?.[ko ? 'ko' : 'en']} · {percent}%</span><small>{ko ? '문서 내용과 전체 경로는 진행 기록에 포함되지 않습니다.' : 'Document content and full paths are excluded from progress records.'}</small></div>}
    {error && <div role="alert" className="document-error"><p>{ko ? `문서 번역 실패: ${error}` : `Document translation failed: ${error}`}</p><button type="button" onClick={() => void start()} disabled={!chosen || busy}>{ko ? '다시 시도' : 'Retry'}</button></div>}
    {report && <DocumentReport locale={locale} report={report} onRetry={() => void start()}/>}</section>;
}
