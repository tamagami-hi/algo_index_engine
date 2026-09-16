import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ResolutionView } from "../../src/pages/Execution";
import { resolution } from "../support/factories";

describe("entry and session are separate permissions", () => {
  it("can show entry shut while the session is still live", () => {
    render(
      <ResolutionView
        resolution={resolution({
          entry_window_open: false,
          before_hard_exit: true,
          entry_closes_at: "09:17",
        })}
      />,
    );

    expect(screen.getByText(/entry shut/)).toBeInTheDocument();
    expect(screen.getByText("session live")).toBeInTheDocument();
  });

  it("shows entry open during the entry minute", () => {
    render(
      <ResolutionView
        resolution={resolution({ entry_window_open: true, before_hard_exit: true })}
      />,
    );

    expect(screen.getByText("entry window open")).toBeInTheDocument();
    expect(screen.getByText("session live")).toBeInTheDocument();
  });

  it("shows past hard exit once the session is over", () => {
    render(
      <ResolutionView
        resolution={resolution({ entry_window_open: false, before_hard_exit: false })}
      />,
    );

    expect(screen.getByText(/entry shut/)).toBeInTheDocument();
    expect(screen.getByText("past hard exit")).toBeInTheDocument();
  });

  it("names the minute entry closed at, so a late start is explicable", () => {
    render(
      <ResolutionView
        resolution={resolution({ entry_window_open: false, entry_closes_at: "09:17" })}
      />,
    );
    expect(screen.getByText(/was until 09:17/)).toBeInTheDocument();
  });
});

describe("readiness reasons", () => {
  it("lists why a strategy will not enter rather than only saying it will not", () => {
    render(
      <ResolutionView
        resolution={resolution({
          would_enter_now: false,
          blocked_because: [
            "the market feed is not connected",
            "leg 1 (CE) has no usable bid",
          ],
        })}
      />,
    );

    expect(screen.getByText("would not enter")).toBeInTheDocument();
    const blockers = screen.getByTestId("blockers");
    expect(blockers.textContent).toContain("the market feed is not connected");
    expect(blockers.textContent).toContain("leg 1 (CE) has no usable bid");
  });

  it("shows no blocker list when the strategy is ready", () => {
    render(
      <ResolutionView
        resolution={resolution({ would_enter_now: true, blocked_because: [] })}
      />,
    );

    expect(screen.getByText("would enter now")).toBeInTheDocument();
    expect(screen.queryByTestId("blockers")).not.toBeInTheDocument();
  });

  it("reports how old the underlying quote is", () => {
    render(<ResolutionView resolution={resolution({ spot_age_ms: 1_500 })} />);
    expect(screen.getByText(/spot 1.5s old/)).toBeInTheDocument();
  });

  it("says so plainly when the underlying has never quoted", () => {
    render(<ResolutionView resolution={resolution({ spot_age_ms: null })} />);
    expect(screen.getByText(/spot never quoted/)).toBeInTheDocument();
  });
});
