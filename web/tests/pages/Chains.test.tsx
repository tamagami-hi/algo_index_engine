import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { Chains } from "../../src/pages/Chains";
import { useEngine } from "../../src/stores/engine";
import { chain, side } from "../support/factories";

const CALL_BID = 3;
const CALL_ASK = 4;
const CALL_LTP = 5;
const CALL_CHANGE_OI = 1;

function seed(chainValue: ReturnType<typeof chain> | null) {
  useEngine.setState({
    chain: chainValue,
    symbols: ["NIFTY", "BANKNIFTY"],
    selected: "NIFTY",
    link: "open",
    synchronised: true,
  });
}

function cells(row: number): string[] {
  const body = screen.getByRole("table").querySelectorAll("tbody tr");
  const found = body[row];
  if (!found) {
    throw new Error(`no row ${row}`);
  }
  return Array.from(found.querySelectorAll("td")).map((cell) => cell.textContent ?? "");
}

describe("cleared liquidity", () => {
  it("shows a real two-sided quote", () => {
    seed(chain());
    render(<Chains />);

    const row = cells(0);
    expect(row[CALL_BID]).toBe("128.80");
    expect(row[CALL_ASK]).toBe("129.80");
    expect(row[CALL_LTP]).toBe("129.30");
  });

  it("shows an em dash rather than the old price when a bid disappears", () => {
    seed(chain({ call: side({ bid: [null, 87.0], bid_quantity: [0, 600] }) }));
    render(<Chains />);

    const row = cells(0);
    expect(
      row[CALL_BID],
      "a cleared bid must never keep rendering the price it used to have",
    ).toBe("—");
    expect(row[CALL_ASK], "the ask was still quoted and must survive").toBe("129.80");
  });

  it("does not render a cleared price as a zero that looks like a real price", () => {
    seed(chain({ call: side({ bid: [null, null], ask: [null, null] }) }));
    render(<Chains />);

    const row = cells(0);
    expect(row[CALL_BID]).toBe("—");
    expect(row[CALL_ASK]).toBe("—");
    expect(
      row[CALL_BID],
      "0.00 reads as a price of nothing, which is not the same as no price",
    ).not.toBe("0.00");
  });

  it("keeps a genuine zero for counts, where zero is a real value", () => {
    seed(chain({ call: side({ change_in_oi: [0, 0] }) }));
    render(<Chains />);
    expect(cells(0)[CALL_CHANGE_OI]).toBe("0");
  });

  it("restores the display when the quote comes back", () => {
    seed(chain({ call: side({ bid: [null, 87.0] }) }));
    const { unmount } = render(<Chains />);
    expect(cells(0)[CALL_BID]).toBe("—");
    unmount();

    seed(chain());
    render(<Chains />);
    expect(cells(0)[CALL_BID]).toBe("128.80");
  });
});

describe("waiting for data", () => {
  it("says it is waiting rather than showing an empty table", () => {
    seed(null);
    render(<Chains />);
    expect(screen.getByText(/waiting for NIFTY/)).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("offers the other indices for navigation", () => {
    seed(chain());
    render(<Chains />);
    const picker = screen.getByRole("combobox");
    expect(within(picker).getByRole("option", { name: "BANKNIFTY" })).toBeInTheDocument();
  });
});
