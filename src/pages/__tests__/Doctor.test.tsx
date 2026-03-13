import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import doctorImage from "@/assets/doctor.png";
import i18n from "@/i18n";
import { InstanceContext } from "@/lib/instance-context";
import { Doctor } from "../Doctor";

describe("Doctor page rescue header", () => {
  test("centers the doctor image header, shows diagnose button and logs icon", async () => {
    await i18n.changeLanguage("en");
    const storage = new Map<string, string>();
    const originalWindow = (globalThis as { window?: unknown }).window;
    (globalThis as { window?: unknown }).window = {
      localStorage: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => {
          storage.set(key, value);
        },
        removeItem: (key: string) => {
          storage.delete(key);
        },
        clear: () => {
          storage.clear();
        },
      },
    };

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceContext.Provider, {
          value: {
            instanceId: "local",
            instanceViewToken: "local",
            instanceToken: 0,
            persistenceScope: "local",
            persistenceResolved: true,
            isRemote: false,
            isDocker: false,
            isConnected: true,
            channelNodes: null,
            discordGuildChannels: null,
            channelsLoading: false,
            discordChannelsLoading: false,
            refreshChannelNodesCache: async () => [],
            refreshDiscordChannelsCache: async () => [],
          },
          children: React.createElement(Doctor, {}),
        }),
      }),
    );

    expect(html).toContain("flex flex-col items-center");
    expect(html).toContain("role=\"img\"");
    expect(html).toContain("alt=\"Diagnose\"");
    expect(html).toContain(`src="${doctorImage}"`);
    expect(html).toContain("aria-label=\"Open logs\"");
    expect(html).toContain(">Diagnose<");
    expect(html).toContain("Run a structured check before attempting repairs on the primary profile.");
    expect(html).not.toContain("Rescue Bot");
    expect(html).not.toContain("Activate Rescue Bot");
    expect(i18n.t("doctor.diagnose")).toBe("Diagnose");
    expect(i18n.t("doctor.configureTempProvider")).toBe("Configure temp gateway provider");

    (globalThis as { window?: unknown }).window = originalWindow;
  });
});
