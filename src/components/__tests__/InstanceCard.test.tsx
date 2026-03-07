import { describe, expect, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";

import i18n from "@/i18n";
import { InstanceCard } from "../InstanceCard";

describe("InstanceCard SSH connection profile", () => {
  test("shows only connection quality inline", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:hetzner",
          label: "hetzner",
          type: "ssh",
          healthy: false,
          agentCount: 1,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
          sshConnectionProfile: {
            status: {
              healthy: false,
              activeAgents: 1,
              sshDiagnostic: null,
            },
            connectLatencyMs: 120,
            gatewayLatencyMs: 90,
            configLatencyMs: 2420,
            versionLatencyMs: 250,
            totalLatencyMs: 2420,
            quality: "poor",
            qualityScore: 18,
            bottleneck: {
              stage: "config",
              latencyMs: 2420,
            },
          },
        }),
      }),
    );

    expect(html).toContain(">Poor<");
    expect(html).not.toContain("Poor · 2.42 s");
  });

  test("hides quick diagnose button until card hover", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:hetzner",
          label: "hetzner",
          type: "ssh",
          healthy: true,
          agentCount: 1,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
          onQuickDiagnose: () => {},
        }),
      }),
    );

    expect(html).toContain('aria-label="Quick Diagnose"');
    expect(html).toContain("opacity-0");
    expect(html).toContain("group-hover:opacity-100");
  });

  test("uses an expanded click target for the ssh capability dot", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:hetzner",
          label: "hetzner",
          type: "ssh",
          healthy: true,
          agentCount: 1,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
          sshConnectionProfile: {
            status: {
              healthy: true,
              activeAgents: 1,
              sshDiagnostic: null,
            },
            connectLatencyMs: 40,
            gatewayLatencyMs: 50,
            configLatencyMs: 60,
            versionLatencyMs: 35,
            totalLatencyMs: 60,
            quality: "good",
            qualityScore: 82,
            bottleneck: {
              stage: "config",
              latencyMs: 60,
            },
          },
        }),
      }),
    );

    expect(html).toContain('aria-label="Good"');
    expect(html).toContain("-m-2");
    expect(html).toContain("p-2");
  });

  test("makes the ssh capability label part of the popover trigger", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:hetzner",
          label: "hetzner",
          type: "ssh",
          healthy: false,
          agentCount: 1,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
          sshConnectionProfile: {
            status: {
              healthy: false,
              activeAgents: 1,
              sshDiagnostic: null,
            },
            connectLatencyMs: 120,
            gatewayLatencyMs: 90,
            configLatencyMs: 2420,
            versionLatencyMs: 250,
            totalLatencyMs: 2420,
            quality: "poor",
            qualityScore: 18,
            bottleneck: {
              stage: "config",
              latencyMs: 2420,
            },
          },
        }),
      }),
    );

    expect(html).toContain("inline-flex items-center gap-1.5");
    expect(html).toContain(">Poor</span></button>");
  });

  test("shows a check button before an ssh host has been checked", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:new-host",
          label: "new-host",
          type: "ssh",
          healthy: null,
          agentCount: 0,
          opened: false,
          checked: false,
          checking: false,
          onCheck: () => {},
          onClick: () => {},
        }),
      }),
    );

    expect(html).toContain(">Check<");
    expect(html).toContain("lucide-refresh-cw");
  });

  test("shows checking state while a check is running without an opened badge", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:checking",
          label: "checking",
          type: "ssh",
          healthy: null,
          agentCount: 2,
          opened: true,
          checked: false,
          checking: true,
          onClick: () => {},
        }),
      }),
    );

    expect(html).toContain(">Checking...</span>");
    expect(html).not.toContain(">Open<");
    expect(html).toContain("animate-spin");
  });

  test("renders a local not-installed summary without health or agent badges", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "local",
          label: "Local",
          type: "local",
          healthy: false,
          agentCount: 1,
          opened: false,
          notInstalled: true,
          onClick: () => {},
        }),
      }),
    );

    expect(html).toContain(">Not installed<");
    expect(html).not.toContain(">Unhealthy<");
    expect(html).not.toContain(">1 agent<");
  });

  test("renders discovered instance source and connect call to action", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "docker:detected",
          label: "detected",
          type: "docker",
          healthy: true,
          agentCount: 1,
          opened: false,
          discovered: true,
          discoveredSource: "container",
          onConnect: () => {},
          onClick: () => {},
        }),
      }),
    );

    expect(html).toContain(">Docker<");
    expect(html).toContain("From Docker container");
    expect(html).toContain(">Connect<");
  });

  test("renders the unreachable state for offline checked hosts", async () => {
    await i18n.changeLanguage("en");

    const html = renderToStaticMarkup(
      React.createElement(I18nextProvider, {
        i18n,
        children: React.createElement(InstanceCard, {
          id: "ssh:offline",
          label: "offline",
          type: "ssh",
          healthy: null,
          agentCount: 0,
          opened: false,
          checked: true,
          checking: false,
          onClick: () => {},
        }),
      }),
    );

    expect(html).toContain(">Unreachable<");
    expect(html).toContain("bg-muted-foreground/40");
  });
});
