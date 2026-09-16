import { beforeEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { LinkStatus, StaleBanner } from "./LinkStatus";
import { useEngine } from "../stores/engine";
import type { Link } from "../stores/engine";
import { frame } from "../test/factories";

function seed(overrides: Partial<ReturnType<typeof useEngine.getState>> = {}) {
  useEngine.setState({
    snapshot: null,
    chain: null,
    selected: "NIFTY",
    symbols: [],
    link: "connecting",
    synchronised: false,
    sequence: null,
    gaps: 0,
    feedAgeMs: null,
    feedSilenceLimitMs: null,
    publishIntervalMs: null,
    receivedAt: null,
    readyReasons: [],
    error: null,
    ...overrides,
  });
}

describe("connection status display", () => {
  beforeEach(() => seed());

  it("shows live only when the stream is open, synchronised and fresh", () => {
    seed({
      link: "open",
      synchronised: true,
      feedAgeMs: 40,
      feedSilenceLimitMs: 5000,
      receivedAt: Date.now(),
    });
    render(<LinkStatus />);
    expect(screen.getByTestId("link-status")).toHaveAttribute("data-status", "live");
    expect(screen.getByTestId("link-status")).toHaveAttribute("data-stale", "false");
  });

  it("shows each failure state distinctly", () => {
    const cases: [Link, string][] = [
      ["connecting", "connecting"],
      ["reconnecting", "reconnecting"],
      ["unavailable", "backend unavailable"],
      ["not_ready", "backend not ready"],
    ];

    for (const [link, label] of cases) {
      seed({ link, synchronised: true, receivedAt: Date.now() });
      const { unmount } = render(<LinkStatus />);
      expect(screen.getByTestId("link-status")).toHaveAttribute("data-status", label);
      unmount();
    }
  });

  it("marks the data stale when the feed is older than the server's limit", () => {
    seed({
      link: "open",
      synchronised: true,
      feedAgeMs: 9_000,
      feedSilenceLimitMs: 5_000,
      receivedAt: Date.now(),
    });
    render(<LinkStatus />);
    expect(screen.getByTestId("link-status")).toHaveAttribute("data-status", "data stale");
    expect(screen.getByTestId("link-status")).toHaveAttribute("data-stale", "true");
  });

  it("reports dropped sequences so coalescing gaps are visible", () => {
    seed({ link: "open", synchronised: true, receivedAt: Date.now(), gaps: 3 });
    render(<LinkStatus />);
    expect(screen.getByTestId("gap-count").textContent).toContain("3 gaps");
  });
});

describe("stale banner", () => {
  beforeEach(() => seed());

  it("stays out of the way when the data is live", () => {
    seed({
      link: "open",
      synchronised: true,
      feedAgeMs: 20,
      feedSilenceLimitMs: 5000,
      receivedAt: Date.now(),
    });
    render(<StaleBanner />);
    expect(screen.queryByTestId("stale-banner")).not.toBeInTheDocument();
  });

  it("warns explicitly that the numbers are not the current market", () => {
    seed({ link: "reconnecting", synchronised: false, receivedAt: Date.now() });
    render(<StaleBanner />);

    const banner = screen.getByTestId("stale-banner");
    expect(banner.textContent).toContain("reconnecting");
    expect(banner.textContent).toContain("not live");
  });

  it("lists why the backend says it is not ready", () => {
    seed({
      link: "not_ready",
      synchronised: true,
      receivedAt: Date.now(),
      readyReasons: ["the market feed is not connected", "no option chains are assembled"],
    });
    render(<StaleBanner />);

    const banner = screen.getByTestId("stale-banner");
    expect(banner.textContent).toContain("the market feed is not connected");
    expect(banner.textContent).toContain("no option chains are assembled");
  });

  it("goes stale on its own when the stream stops delivering", () => {
    seed({
      link: "open",
      synchronised: true,
      feedAgeMs: 20,
      feedSilenceLimitMs: 5000,
      receivedAt: Date.now() - 10_000,
    });
    render(<StaleBanner />);
    expect(
      screen.getByTestId("stale-banner"),
      "the last frame claimed freshness, but nothing has arrived for ten seconds",
    ).toBeInTheDocument();
  });
});

describe("live updates", () => {
  it("clears the stale banner once a fresh frame arrives", async () => {
    seed({ link: "reconnecting", synchronised: false });
    render(<StaleBanner />);
    expect(screen.getByTestId("stale-banner")).toBeInTheDocument();

    await act(async () => {
      useEngine.getState().applyFrame(frame());
      useEngine.setState({ synchronised: true });
    });

    expect(screen.queryByTestId("stale-banner")).not.toBeInTheDocument();
  });
});
