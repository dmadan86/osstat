/**
 * The chat page.
 *
 * Every test here asserts on content rather than presence. A test that only
 * checked "a meter is on screen" would pass with the numerator and the
 * denominator swapped — the defect found twice already in this repo — so the
 * context test names both figures, and the code-block test names the tag the
 * text landed in rather than that some element exists.
 *
 * Tauri is mocked at `../lib/ipc` the way `Settings.runtime.test.tsx` does it:
 * the subscription functions capture the handler they were given, and a test
 * fires it. Nothing here starts a server or downloads a model (ADR-012).
 */

import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Chat } from './Chat';
import type { ChatComplete } from '../bindings/ChatComplete';
import type { ChatFailure } from '../bindings/ChatFailure';
import type { ChatToken } from '../bindings/ChatToken';
import type { Conversation } from '../bindings/Conversation';
import type { ModelSession } from '../bindings/ModelSession';

const {
  chatClose,
  chatList,
  chatOpenModel,
  chatSend,
  chatStop,
  onChatComplete,
  onChatFailed,
  onChatToken,
} = vi.hoisted(() => ({
  chatClose: vi.fn(),
  chatList: vi.fn(),
  chatOpenModel: vi.fn(),
  chatSend: vi.fn(),
  chatStop: vi.fn(),
  onChatComplete: vi.fn(),
  onChatFailed: vi.fn(),
  onChatToken: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  chatClose,
  chatList,
  chatOpenModel,
  chatSend,
  chatStop,
  onChatComplete,
  onChatFailed,
  onChatToken,
}));

/** Captures the handler a subscription was given, so a test can fire it. */
function capturing<T>(): {
  fire: (payload: T) => void;
  subscribe: (h: (p: T) => void) => Promise<() => void>;
} {
  let handler: ((payload: T) => void) | null = null;

  return {
    fire: (payload) => handler?.(payload),
    subscribe: (h) => {
      handler = h;
      return Promise.resolve(() => {
        handler = null;
      });
    },
  };
}

const MODEL_NAME = 'Mistral-7B-Q4_K_M';

/** The conversation the page resumes, so the tests know its identifier. */
const STORED: Conversation = {
  id: 'c1',
  title: 'About tea',
  modelName: MODEL_NAME,
  messages: [],
};

function session(overrides: Partial<ModelSession> = {}): ModelSession {
  return {
    modelName: MODEL_NAME,
    gpuLayers: 32,
    contextLength: 8192,
    fits: true,
    headDimDerived: false,
    ...overrides,
  };
}

let token = capturing<ChatToken>();
let complete = capturing<ChatComplete>();
let failed = capturing<ChatFailure>();

beforeEach(() => {
  vi.clearAllMocks();

  token = capturing<ChatToken>();
  complete = capturing<ChatComplete>();
  failed = capturing<ChatFailure>();

  chatOpenModel.mockResolvedValue(session());
  chatSend.mockResolvedValue(undefined);
  chatStop.mockResolvedValue(undefined);
  chatClose.mockResolvedValue(undefined);
  chatList.mockResolvedValue([STORED]);
  onChatToken.mockImplementation(token.subscribe);
  onChatComplete.mockImplementation(complete.subscribe);
  onChatFailed.mockImplementation(failed.subscribe);
});

/**
 * Renders the page with a model already open on the stored conversation.
 *
 * Driving the real open flow rather than injecting a session: the page only
 * has a conversation to attribute events to once it has resumed one, and a
 * helper that skipped that would test a state the app cannot reach.
 */
async function renderChat(overrides: Partial<ModelSession> = {}): Promise<void> {
  chatOpenModel.mockResolvedValue(session(overrides));
  render(<Chat />);

  fireEvent.change(await screen.findByLabelText(/model file/i), {
    target: { value: 'C:\\models\\model.gguf' },
  });
  fireEvent.click(screen.getByRole('button', { name: /open model/i }));

  await screen.findByText(MODEL_NAME);
  await screen.findByText(STORED.title);
}

/** Fires one event and lets React settle. */
async function emit<T>(channel: { fire: (payload: T) => void }, payload: T): Promise<void> {
  await act(async () => {
    channel.fire(payload);
    await Promise.resolve();
  });
}

describe('Chat', () => {
  it('renders tokens as they arrive rather than only at the end', async () => {
    await renderChat();

    await emit(token, { conversationId: 'c1', delta: 'Hel', timings: null });
    await emit(token, { conversationId: 'c1', delta: 'lo', timings: null });

    expect(await screen.findByText('Hello')).toBeInTheDocument();
  });

  it('fills the context meter from usage against the window', async () => {
    await renderChat({ contextLength: 4096 });

    await emit(complete, {
      conversationId: 'c1',
      usage: { promptTokens: 1024, completionTokens: 24 },
      timings: null,
      stopped: false,
    });

    // The detail line, not just the meter's presence: a swapped numerator
    // would still render a meter.
    expect(await screen.findByText(/1,048 of 4,096 tokens/)).toBeInTheDocument();
  });

  it('shows generation speed while streaming and stops showing it after', async () => {
    await renderChat();

    await emit(token, {
      conversationId: 'c1',
      delta: 'hi',
      timings: { promptPerSecond: 100, predictedPerSecond: 52.9 },
    });
    expect(await screen.findByText(/52\.9 tok\/s/)).toBeInTheDocument();

    await emit(complete, {
      conversationId: 'c1',
      usage: { promptTokens: 7, completionTokens: 1 },
      timings: null,
      stopped: false,
    });

    await waitFor(() => {
      expect(screen.queryByText(/tok\/s/)).not.toBeInTheDocument();
    });
  });

  it('labels each exchange with its own token counts', async () => {
    await renderChat();

    await emit(complete, {
      conversationId: 'c1',
      usage: { promptTokens: 44, completionTokens: 48 },
      timings: null,
      stopped: false,
    });

    expect(await screen.findByText(/44 in · 48 out/)).toBeInTheDocument();
  });

  it('ignores events belonging to another conversation', async () => {
    // The identifier on every payload is the only thing separating two
    // conversations. A page that appended whatever arrived would splice one
    // model's answer into another conversation's transcript.
    await renderChat();

    await emit(token, { conversationId: 'c2', delta: 'not yours', timings: null });

    expect(screen.queryByText('not yours')).not.toBeInTheDocument();
  });

  it('surfaces a dead session instead of hanging', async () => {
    await renderChat();

    await emit(failed, {
      conversationId: 'c1',
      message: 'llama-server exited: ggml_backend_cuda_buffer_type_alloc: out of memory',
    });

    expect(await screen.findByText(/out of memory/)).toBeInTheDocument();
    // The conversation must survive the crash -- losing it would punish the
    // user for the model being too big.
    expect(screen.getByRole('textbox')).toBeInTheDocument();
  });

  it('keeps partial text when generation is stopped', async () => {
    await renderChat();

    await emit(token, { conversationId: 'c1', delta: 'Once upon a', timings: null });
    await userEvent.click(screen.getByRole('button', { name: /stop/i }));

    expect(await screen.findByText('Once upon a')).toBeInTheDocument();
    expect(screen.getByText(/stopped/i)).toBeInTheDocument();
    expect(chatStop).toHaveBeenCalled();
  });

  it('renders a fenced code block as code rather than as prose', async () => {
    await renderChat();

    await emit(token, {
      conversationId: 'c1',
      delta: 'try:\n```rust\nfn main() {}\n```\ndone',
      timings: null,
    });

    const code = await screen.findByText('fn main() {}');
    expect(code.tagName).toBe('CODE');
    // The prose around the fence stays prose.
    expect(screen.getByText('try:').tagName).not.toBe('CODE');
  });

  it('does not render markdown emphasis as HTML', async () => {
    // Plain text is the deliberate choice. Asserting it stays plain is what
    // stops a later "small improvement" from introducing an unsanitised path.
    await renderChat();

    await emit(token, {
      conversationId: 'c1',
      delta: '<b>bold</b> and **stars**',
      timings: null,
    });

    const rendered = await screen.findByText(/<b>bold<\/b> and \*\*stars\*\*/);
    expect(rendered.querySelector('b')).toBeNull();
  });
});
