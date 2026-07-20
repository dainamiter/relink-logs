import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/updater", () => ({ checkUpdate: vi.fn(), installUpdate: vi.fn() }));
vi.mock("@mantine/modals", () => ({ modals: { openConfirmModal: vi.fn() } }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: unknown) => (typeof fallback === "string" ? fallback : key),
    i18n: { language: "en" },
  }),
}));

import { modals } from "@mantine/modals";
import { checkUpdate, installUpdate } from "@tauri-apps/api/updater";

import { useMeterSettingsStore } from "@/stores/useMeterSettingsStore";
import useUpdateCheck from "./useUpdateCheck";

const checkUpdateMock = vi.mocked(checkUpdate);
const installUpdateMock = vi.mocked(installUpdate);
const openConfirmModalMock = vi.mocked(modals.openConfirmModal);

const update = (shouldUpdate: boolean) =>
  ({ shouldUpdate, manifest: { version: "1.11.0", date: "", body: "- Something new" } }) as Awaited<
    ReturnType<typeof checkUpdate>
  >;

describe("auto_check_updates setting", () => {
  it("defaults to on", () => {
    expect(useMeterSettingsStore.getState().auto_check_updates).toBe(true);
  });
});

describe("useUpdateCheck", () => {
  beforeEach(() => {
    checkUpdateMock.mockReset();
    installUpdateMock.mockReset();
    openConfirmModalMock.mockReset();
  });

  it("does not contact the update endpoint when disabled", () => {
    renderHook(() => useUpdateCheck(false));
    expect(checkUpdateMock).not.toHaveBeenCalled();
  });

  it("prompts when an update is available, and installs on confirm", async () => {
    checkUpdateMock.mockResolvedValue(update(true));
    installUpdateMock.mockResolvedValue(undefined);
    renderHook(() => useUpdateCheck(true));
    await waitFor(() => expect(openConfirmModalMock).toHaveBeenCalledTimes(1));
    const args = openConfirmModalMock.mock.calls[0][0];
    await act(async () => args.onConfirm?.());
    expect(installUpdateMock).toHaveBeenCalledTimes(1);
  });

  it("does not prompt when the app is up to date", async () => {
    checkUpdateMock.mockResolvedValue(update(false));
    renderHook(() => useUpdateCheck(true));
    await waitFor(() => expect(checkUpdateMock).toHaveBeenCalledTimes(1));
    expect(openConfirmModalMock).not.toHaveBeenCalled();
  });

  it("prompts at most once even when the setting is toggled off and on", async () => {
    checkUpdateMock.mockResolvedValue(update(true));
    const { rerender } = renderHook(({ enabled }) => useUpdateCheck(enabled), {
      initialProps: { enabled: true },
    });
    await waitFor(() => expect(openConfirmModalMock).toHaveBeenCalledTimes(1));
    rerender({ enabled: false });
    rerender({ enabled: true });
    await waitFor(() => expect(checkUpdateMock.mock.calls.length).toBeGreaterThanOrEqual(1));
    expect(openConfirmModalMock).toHaveBeenCalledTimes(1);
  });

  it("swallows endpoint failures (offline, missing manifest)", async () => {
    checkUpdateMock.mockRejectedValue(new Error("offline"));
    renderHook(() => useUpdateCheck(true));
    await waitFor(() => expect(checkUpdateMock).toHaveBeenCalledTimes(1));
    expect(openConfirmModalMock).not.toHaveBeenCalled();
  });
});
