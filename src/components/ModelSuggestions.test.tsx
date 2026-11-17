import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { RankedModel } from "../lib/types";
import { ModelSuggestions } from "./ModelSuggestions";

const rankedModels = vi.fn<(category: string) => Promise<RankedModel[]>>();

vi.mock("../lib/ipc", () => ({
  api: { rankedModels: (category: string) => rankedModels(category) },
}));

function model(over: Partial<RankedModel> = {}): RankedModel {
  return {
    id: "openai/gpt-5.6-sol",
    name: "OpenAI: GPT-5.6 Sol",
    contextLength: 400_000,
    promptPerMillion: 2,
    completionPerMillion: 12,
    ...over,
  };
}

const THREE = [
  model(),
  model({ id: "x-ai/grok-4.6", name: "xAI: Grok 4.6", promptPerMillion: 3 }),
  model({ id: "z-ai/glm-5.2", name: "Z.AI: GLM 5.2", promptPerMillion: 0.6 }),
];

function draw(over: Partial<Parameters<typeof ModelSuggestions>[0]> = {}) {
  const onChoose = vi.fn();
  render(
    <ModelSuggestions
      name="Counsel"
      skills={["contract review"]}
      instructions=""
      model=""
      active={true}
      onChoose={onChoose}
      {...over}
    />,
  );
  return onChoose;
}

describe("ModelSuggestions", () => {
  beforeEach(() => {
    rankedModels.mockReset();
    rankedModels.mockResolvedValue(THREE);
  });

  it("asks for the use case the agent reads as, and offers what comes back", async () => {
    draw();

    await waitFor(() => expect(rankedModels).toHaveBeenCalledWith("legal"));
    expect(await screen.findByText("OpenAI: GPT-5.6 Sol")).toBeTruthy();
    expect(screen.getByText("x-ai/grok-4.6")).toBeTruthy();
    expect(screen.getByText(/reads as legal work/i)).toBeTruthy();
  });

  it("puts the slug in the field and nothing else", async () => {
    const onChoose = draw();

    fireEvent.click(
      await screen.findByRole("button", { name: "Use xAI: Grok 4.6 for legal work" }),
    );

    expect(onChoose).toHaveBeenCalledWith("x-ai/grok-4.6");
    expect(onChoose).toHaveBeenCalledTimes(1);
  });

  it("says nothing when OpenRouter is not what pays", () => {
    draw({ active: false });

    expect(rankedModels).not.toHaveBeenCalled();
    expect(screen.queryByText(/reads as/i)).toBeNull();
  });

  it("says nothing about an agent that is not one of the twelve", () => {
    draw({ name: "Manager", skills: [], instructions: "You coordinate the others." });

    expect(rankedModels).not.toHaveBeenCalled();
    expect(screen.queryByText(/reads as/i)).toBeNull();
  });

  it("does not offer the model already in the field", async () => {
    draw({ model: "openai/gpt-5.6-sol" });

    expect(await screen.findByText("xAI: Grok 4.6")).toBeTruthy();
    expect(screen.queryByText("OpenAI: GPT-5.6 Sol")).toBeNull();
  });

  it("offers three at most, however many come back", async () => {
    rankedModels.mockResolvedValue([...THREE, model({ id: "a/b", name: "A: B" })]);
    draw();

    await screen.findByText("OpenAI: GPT-5.6 Sol");
    expect(screen.getAllByRole("button")).toHaveLength(3);
  });

  it("prices every row", async () => {
    draw();

    expect(await screen.findByText("$2.00 / $12.00")).toBeTruthy();
    expect(screen.getByText("$0.60 / $12.00")).toBeTruthy();
  });

  it("tells a free model apart from one that quoted no price", async () => {
    rankedModels.mockResolvedValue([
      model({ id: "free/one", name: "Free One", promptPerMillion: 0, completionPerMillion: 0 }),
      model({
        id: "quiet/one",
        name: "Quiet One",
        promptPerMillion: null,
        completionPerMillion: null,
      }),
    ]);
    draw();

    expect(await screen.findByText("free")).toBeTruthy();
    expect(screen.getByText("no price")).toBeTruthy();
  });

  it("floors a fraction of a cent rather than spelling it out", async () => {
    rankedModels.mockResolvedValue([
      model({ id: "cheap/one", name: "Cheap One", promptPerMillion: 0.0000488 }),
    ]);
    draw();

    expect(await screen.findByText("<$0.01 / $12.00")).toBeTruthy();
  });

  it("says so quietly when OpenRouter does not answer", async () => {
    rankedModels.mockRejectedValue(new Error("no route to host"));
    draw();

    expect(await screen.findByText(/did not answer/i)).toBeTruthy();
    expect(screen.getByText(/type a slug as usual/i)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("asks once while the evidence is still being typed", async () => {
    const { rerender } = render(
      <ModelSuggestions
        name="Leg"
        skills={["contract review"]}
        instructions=""
        model=""
        active={true}
        onChoose={vi.fn()}
      />,
    );
    await waitFor(() => expect(rankedModels).toHaveBeenCalledTimes(1));

    for (const name of ["Lega", "Legal", "Legal Counsel"]) {
      rerender(
        <ModelSuggestions
          name={name}
          skills={["contract review"]}
          instructions=""
          model=""
          active={true}
          onChoose={vi.fn()}
        />,
      );
    }

    expect(rankedModels).toHaveBeenCalledTimes(1);
    expect(rankedModels).toHaveBeenCalledWith("legal");
  });

  it("asks again when the agent starts reading as something else", async () => {
    const { rerender } = render(
      <ModelSuggestions
        name="Counsel"
        skills={[]}
        instructions=""
        model=""
        active={true}
        onChoose={vi.fn()}
      />,
    );
    await waitFor(() => expect(rankedModels).toHaveBeenCalledWith("legal"));

    rerender(
      <ModelSuggestions
        name="Accountant"
        skills={[]}
        instructions=""
        model=""
        active={true}
        onChoose={vi.fn()}
      />,
    );

    await waitFor(() => expect(rankedModels).toHaveBeenCalledWith("finance"));
    expect(rankedModels).toHaveBeenCalledTimes(2);
  });
});
