import { invoke } from '@tauri-apps/api/core';

export type InstallableUpdate = {
  version: string;
  consentToken: string;
};

export async function installSignedUpdate(update: InstallableUpdate): Promise<void> {
  const prepared = await invoke<{ installToken: string }>('prepare_update', {
    version: update.version,
    consentToken: update.consentToken,
  });
  const consent = await invoke<{ restartConsentToken: string }>('authorize_update_restart', {
    version: update.version,
    installToken: prepared.installToken,
  });
  await invoke('install_update', {
    version: update.version,
    installToken: prepared.installToken,
    restartConsentToken: consent.restartConsentToken,
  });
}
