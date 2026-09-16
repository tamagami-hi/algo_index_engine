import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrategyEditor } from "../../src/components/StrategyEditor";
import { useStrategies } from "../../src/stores/strategies";
import { strategy } from "../support/factories";
import { MAX_PREMIUM_PERCENT } from "../../src/types/api";

function legScope(index: number): HTMLElement {
  const legend = screen.getByText(new RegExp(`^leg ${index} ·`));
  const fieldset = legend.closest("fieldset");
  if (!fieldset) {
    throw new Error(`no fieldset for leg ${index}`);
  }
  return fieldset as HTMLElement;
}

function fieldFor(label: string, scope: HTMLElement = legScope(1)): HTMLElement {
  const node = within(scope).getByText(label).closest(".field");
  if (!node) {
    throw new Error(`no field labelled ${label}`);
  }
  return node as HTMLElement;
}

function numberIn(label: string, scope: HTMLElement = legScope(1)): HTMLInputElement {
  const inputs = within(fieldFor(label, scope)).getAllByRole("spinbutton");
  return inputs[0] as HTMLInputElement;
}

describe("premium ceilings", () => {
  beforeEach(() => {
    useStrategies.setState({ error: null });
  });

  it("caps a short leg's profit fields at the premium and leaves its stop uncapped", () => {
    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    expect(numberIn("take profit at").max).toBe(String(MAX_PREMIUM_PERCENT));
    expect(numberIn("trail arms at").max).toBe(String(MAX_PREMIUM_PERCENT));
    expect(
      numberIn("stop loss").max,
      "a sold premium can lose more than it collected, so the stop has no ceiling",
    ).toBe("");
  });

  it("mirrors the ceiling onto the loss side once the leg is a purchase", async () => {
    const user = userEvent.setup();
    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    const action = within(fieldFor("action")).getByRole("combobox");
    await user.selectOptions(action, "buy");

    expect(
      numberIn("stop loss").max,
      "a bought premium cannot lose more than it cost",
    ).toBe(String(MAX_PREMIUM_PERCENT));
    expect(
      numberIn("take profit at").max,
      "but its upside is unbounded",
    ).toBe("");
  });

  it("explains which side of each leg is bounded", () => {
    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );
    expect(
      screen.getAllByText(/Selling this leg collects the premium/).length,
    ).toBeGreaterThan(0);
  });

  it("marks a persisted value above the ceiling instead of silently clamping it", () => {
    const overshoot = strategy();
    overshoot.legs[0]!.target = { method: "percent", value: 150 };

    render(
      <StrategyEditor initial={overshoot} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    expect(
      numberIn("take profit at").value,
      "the stored value stays visible so the operator can correct it",
    ).toBe("150");
    expect(screen.getByText(/above 100% — unreachable/)).toBeInTheDocument();
  });

  it("does not apply the percent ceiling to a point threshold", () => {
    const points = strategy();
    points.legs[0]!.target = { method: "points", value: 250 };

    render(
      <StrategyEditor initial={points} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    expect(numberIn("take profit at").value).toBe("250");
    expect(screen.queryByText(/unreachable/)).not.toBeInTheDocument();
  });
});

describe("whole-position trailing", () => {
  it("loads a stored whole-position trail into its controls", () => {
    const trailing = strategy();
    trailing.overall = {
      target: { method: "percent", value: 60 },
      trailing: {
        arm_at: { method: "percent", value: 45 },
        give_back: { method: "percent", value: 12 },
        start_after_minutes: 20,
      },
    };

    render(
      <StrategyEditor initial={trailing} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    const position = screen.getByText("whole position").closest("fieldset");
    if (!position) {
      throw new Error("no whole position fieldset");
    }
    const scoped = within(position);
    const arms = within(
      scoped.getByText("trail arms at").closest(".field") as HTMLElement,
    ).getAllByRole("spinbutton")[0] as HTMLInputElement;
    const gives = within(
      scoped.getByText("trail gives back").closest(".field") as HTMLElement,
    ).getAllByRole("spinbutton")[0] as HTMLInputElement;
    const delay = within(
      scoped.getByText("trail only after (min)").closest(".field") as HTMLElement,
    ).getAllByRole("spinbutton")[0] as HTMLInputElement;

    expect(arms.value).toBe("45");
    expect(gives.value).toBe("12");
    expect(delay.value).toBe("20");
  });

  it("saves an edited whole-position trail back to the engine", async () => {
    const user = userEvent.setup();
    const save = vi.fn().mockResolvedValue(true);
    useStrategies.setState({ save });

    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    const position = screen.getByText("whole position").closest("fieldset");
    const arms = within(
      within(position as HTMLElement)
        .getByText("trail arms at")
        .closest(".field") as HTMLElement,
    ).getAllByRole("spinbutton")[0] as HTMLInputElement;

    await user.clear(arms);
    await user.type(arms, "45");
    await user.click(screen.getByRole("button", { name: "save strategy" }));

    expect(save).toHaveBeenCalledTimes(1);
    const submitted = save.mock.calls[0]![0];
    expect(submitted.overall.trailing.arm_at).toEqual({
      method: "percent",
      value: 45,
    });
  });
});

describe("rejection messages", () => {
  it("renders a structured engine rejection as a sentence naming leg and field", async () => {
    const user = userEvent.setup();
    useStrategies.setState({
      save: vi.fn().mockResolvedValue(false),
      error:
        'Error: {"error":"strategy is not valid: {\\"problem\\":\\"beyond_premium_ceiling\\",\\"leg\\":0,\\"field\\":\\"target\\",\\"value\\":150.0,\\"ceiling\\":100.0}"}',
    });

    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );
    await user.click(screen.getByRole("button", { name: "save strategy" }));

    const message = await screen.findByText(/leg 1: target is set to 150%/);
    expect(message).toBeInTheDocument();
    expect(message.textContent).toContain("past the 100% the premium can move");
    expect(message.textContent).not.toContain("{");
  });

  it("names the whole position when a rejection has no leg", async () => {
    const user = userEvent.setup();
    useStrategies.setState({
      save: vi.fn().mockResolvedValue(false),
      error:
        '{"problem":"trail_gives_back_more_than_it_captures","leg":null,"arm_at":40.0,"give_back":60.0}',
    });

    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );
    await user.click(screen.getByRole("button", { name: "save strategy" }));

    expect(
      await screen.findByText(/the whole position: the trail arms at 40 but gives back 60/),
    ).toBeInTheDocument();
  });

  it("falls back to the raw message when the problem is unrecognised", async () => {
    const user = userEvent.setup();
    useStrategies.setState({
      save: vi.fn().mockResolvedValue(false),
      error: "something the client has never heard of",
    });

    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );
    await user.click(screen.getByRole("button", { name: "save strategy" }));

    expect(
      await screen.findByText("something the client has never heard of"),
    ).toBeInTheDocument();
  });
});

describe("dte selection", () => {
  it("offers 0 to 6 and an all-DTE toggle", async () => {
    const user = userEvent.setup();
    render(
      <StrategyEditor initial={strategy()} symbols={["NIFTY"]} onClose={() => {}} />,
    );

    for (const day of [0, 1, 2, 3, 4, 5, 6]) {
      expect(screen.getByText(`${day} DTE`)).toBeInTheDocument();
    }
    expect(screen.queryByText("7 DTE")).not.toBeInTheDocument();

    const all = screen.getByText("all DTE").closest("label");
    await user.click(within(all as HTMLElement).getByRole("checkbox"));

    const ticked = screen
      .getAllByRole("checkbox")
      .filter((box) => (box as HTMLInputElement).checked);
    expect(ticked.length, "all seven days plus the master toggle").toBe(8);
  });

  it("warns when no DTE is selected at all", async () => {
    const user = userEvent.setup();
    const none = strategy({ dte: [0] });
    render(<StrategyEditor initial={none} symbols={["NIFTY"]} onClose={() => {}} />);

    const zero = screen.getByText("0 DTE").closest("label");
    await user.click(within(zero as HTMLElement).getByRole("checkbox"));

    expect(
      screen.getByText(/select at least one DTE or the strategy can never run/),
    ).toBeInTheDocument();
  });
});
