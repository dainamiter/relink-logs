import { Text } from "@mantine/core";
import { modals } from "@mantine/modals";
import { checkUpdate, installUpdate } from "@tauri-apps/api/updater";
import { useEffect, useRef } from "react";
import toast from "react-hot-toast";
import { useTranslation } from "react-i18next";

/**
 * When `enabled`, asks the update endpoint once whether a newer release
 * exists and offers it in a confirm dialog showing the release notes
 * (CHANGELOG.md section, via the updater manifest). Confirming downloads and
 * runs the installer — on Windows the app exits into it, so there is nothing
 * to do afterwards. Requires `updater.dialog: false` in tauri.conf.json;
 * failures (offline, endpoint gone) stay silent.
 */
export default function useUpdateCheck(enabled: boolean) {
  const { t } = useTranslation();
  // One prompt per app run, across setting toggles and re-renders.
  const prompted = useRef(false);

  useEffect(() => {
    if (!enabled || prompted.current) return;
    let cancelled = false;
    checkUpdate()
      .then(({ shouldUpdate, manifest }) => {
        if (cancelled || !shouldUpdate || prompted.current) return;
        prompted.current = true;
        modals.openConfirmModal({
          title: t("ui.update-available", { version: manifest?.version }),
          children: (
            <Text size="sm" style={{ whiteSpace: "pre-wrap" }}>
              {manifest?.body}
            </Text>
          ),
          labels: { confirm: t("ui.update-now"), cancel: t("ui.update-later") },
          onConfirm: () => {
            installUpdate().catch(() => toast.error(t("ui.update-failed")));
          },
        });
      })
      .catch(() => {
        // Offline or the endpoint is unreachable — try again next launch.
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, t]);
}
