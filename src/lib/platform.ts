export type DesktopPlatform = 'windows' | 'macos';

type NavigatorWithUserAgentData = Navigator & {
  userAgentData?: { platform?: string };
};

export function detectDesktopPlatform(
  browserNavigator: NavigatorWithUserAgentData = navigator as NavigatorWithUserAgentData,
): DesktopPlatform {
  const identity = [
    browserNavigator.userAgentData?.platform,
    browserNavigator.userAgent,
    browserNavigator.platform,
  ].filter(Boolean).join(' ').toLowerCase();

  return /macintosh|mac os|macintel|macppc/.test(identity) ? 'macos' : 'windows';
}
