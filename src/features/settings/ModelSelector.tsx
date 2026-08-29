import type { AppLocale, AvailableModel, ModelChoice } from './SettingsView';

const copy = {
  ko: { group: '모델', select: '모델 선택', automatic: '자동 선택', warning: '사용할 수 없어 자동 선택으로 전환됨' },
  en: { group: 'Model', select: 'Model selection', automatic: 'Automatic', warning: 'Unavailable; switched to automatic selection' },
} as const;

export function modelChoiceValue(choice: ModelChoice, models: AvailableModel[]) {
  return choice.type === 'specific' && models.some((model) => model.id === choice.id)
    ? choice.id
    : 'automatic';
}

export function ModelSelector({
  locale,
  choice,
  models,
  onChange,
}: {
  locale: AppLocale;
  choice: ModelChoice;
  models: AvailableModel[];
  onChange: (choice: ModelChoice) => void;
}) {
  const labels = copy[locale];
  const missing = choice.type === 'specific' && !models.some((model) => model.id === choice.id);
  return (
    <fieldset>
      <legend>{labels.group}</legend>
      <label>{labels.select}
        <select
          value={modelChoiceValue(choice, models)}
          onChange={(event) => onChange(event.target.value === 'automatic'
            ? { type: 'automatic' }
            : { type: 'specific', id: event.target.value })}
        >
          <option value="automatic">{labels.automatic}</option>
          {models.map((model) => <option key={model.id} value={model.id}>{model.displayName}</option>)}
        </select>
      </label>
      {missing && <p role="status">{labels.warning}</p>}
    </fieldset>
  );
}
