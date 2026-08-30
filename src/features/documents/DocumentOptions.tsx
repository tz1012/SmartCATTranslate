import { chooseDocumentOutputDirectory } from './documentApi';
import type { DocumentFormat, DocumentOptions as Options } from './types';

export function DocumentOptions({ locale, value, format, disabled, onChange }: { locale: 'ko' | 'en'; value: Options; format?: DocumentFormat; disabled: boolean; onChange: (value: Options) => void }) {
  const ko = locale === 'ko'; const patch = (next: Partial<Options>) => onChange({ ...value, ...next });
  return <fieldset disabled={disabled} className="document-options"><legend>{ko ? '번역 옵션' : 'Translation options'}</legend>
    <div className="document-option-grid">
      <label>{ko ? '원문 언어' : 'Source language'}<select value={value.sourceLanguage ?? 'auto'} onChange={(e) => patch({ sourceLanguage: e.target.value === 'auto' ? null : e.target.value })}><option value="auto">{ko ? '자동 감지' : 'Auto detect'}</option><option value="ko">한국어</option><option value="en">English</option><option value="ja">日本語</option><option value="zh">中文</option></select></label>
      <label>{ko ? '대상 언어' : 'Target language'}<select value={value.targetLanguage} onChange={(e) => patch({ targetLanguage: e.target.value })}><option value="ko">한국어</option><option value="en">English</option><option value="ja">日本語</option><option value="zh">中文</option><option value="de">Deutsch</option><option value="fr">Français</option><option value="es">Español</option></select></label>
      <label>{ko ? '모델' : 'Model'}<select value={value.model ?? 'automatic'} onChange={(e) => patch({ model: e.target.value === 'automatic' ? null : e.target.value })}><option value="automatic">{ko ? '계정 기본값' : 'Account default'}</option></select></label>
      <label>{ko ? '프로필·용어집' : 'Profile & glossary'}<select value={value.profileId ?? 'default'} onChange={(e) => patch({ profileId: e.target.value === 'default' ? null : e.target.value })}><option value="default">{ko ? '기본 프로필과 일치 용어집' : 'Default profile and matching glossary'}</option></select></label>
    </div>
    <label><input type="checkbox" checked={value.includeComments} onChange={(e) => patch({ includeComments: e.target.checked })}/>{ko ? '댓글·메모 포함' : 'Include comments'}</label>
    <label><input type="checkbox" checked={value.includeNotes} onChange={(e) => patch({ includeNotes: e.target.checked })}/>{ko ? '발표자 노트 포함' : 'Include speaker notes'}</label>
    <label><input type="checkbox" checked={value.includeHidden} onChange={(e) => patch({ includeHidden: e.target.checked })}/>{ko ? '숨겨진 슬라이드·시트 포함' : 'Include hidden slides and sheets'}</label>
    {format === 'xlsx' && <label><input type="checkbox" checked={value.wrapText} onChange={(e) => patch({ wrapText: e.target.checked })}/>{ko ? '번역 셀 줄 바꿈' : 'Wrap translated cells'}</label>}
    {format === 'pdf' && <><label><input type="checkbox" checked={value.pdfFit} onChange={(e) => patch({ pdfFit: e.target.checked })}/>{ko ? 'PDF 영역에 맞게 글자 크기 조정' : 'Fit PDF text to original areas'}</label><label><input type="checkbox" checked={value.pdfForceOcr} onChange={(e) => patch({ pdfForceOcr: e.target.checked })}/>{ko ? '모든 PDF 페이지를 OCR로 처리' : 'Force OCR for every PDF page'}</label><label><input type="checkbox" checked={value.preserveAnnotations} onChange={(e) => patch({ preserveAnnotations: e.target.checked })}/>{ko ? '링크·주석·양식 보존' : 'Preserve links, annotations and forms'}</label></>}
    <div className="document-destination"><span>{ko ? '저장 위치' : 'Destination'}: {value.outputDirectory ? (ko ? '선택한 폴더' : 'Selected folder') : (ko ? '원본과 같은 폴더' : 'Beside original')}</span><button type="button" onClick={() => void chooseDocumentOutputDirectory().then((directory) => { if (directory) patch({ outputDirectory: directory }); })}>{ko ? '폴더 변경' : 'Change folder'}</button>{value.outputDirectory && <details><summary>{ko ? '경로 보기' : 'Show path'}</summary><code>{value.outputDirectory}</code></details>}</div>
  </fieldset>;
}
