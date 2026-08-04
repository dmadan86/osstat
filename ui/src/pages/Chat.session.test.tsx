/**
 * Who owns the chat session.
 *
 * Rust does, and that is the whole subject of this file. The model is opened by
 * the Run control on the LLM tab *before* this page exists, and it stays open
 * across the page being unmounted and mounted again — React's development
 * double-invoke does exactly that on purpose, and so does navigating away and
 * straight back. A page that treated its own lifetime as the session's drew a
 * model bar for a server it had just closed, and every message was refused.
 *
 * **The page no longer closes anything on unmount.** ADR-013 originally ended
 * the server when the chat page was left; the user asked for the model to
 * survive navigating between tabs, and the reversal is recorded there. So the
 * assertions below are the opposite of the ones they replaced: leaving keeps the
 * session, and the two things that end it are the Unload control and the
 * application exiting.
 *
 * So the mock here is not a set of resolved values but a stand-in for the state
 * Rust holds: `chat_open_model` opens it, `chat_close` closes it,
 * `chat_status` reports it, and `chat_send` refuses in exactly the words the
 * user saw when nothing is open. Mocks that always resolved would pass against
 * a page that had closed the session out from under itself, which is the defect
 * these tests exist to catch.
 */

import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Chat } from './Chat';
import type { ChatComplete } from '../bindings/ChatComplete';
import type { ChatFailure } from '../bindings/ChatFailure';
import type { ChatToken } from '../bindings/ChatToken';
import type { Conversation } from '../bindings/Conversation';
import type { ModelSession } from '../bindings/ModelSession';

const {
  chatClose,
  chatDelete,
  chatList,
  chatLoad,
  chatOpenModel,
  chatSend,
  chatStatus,
  chatStop,
  fetchModelCatalogue,
  onChatComplete,
  onChatFailed,
  onChatToken,
} = vi.hoisted(() => ({
  chatClose: vi.fn(),
  chatDelete: vi.fn(),
  chatList: vi.fn(),
  chatLoad: vi.fn(),
  chatOpenModel: vi.fn(),
  chatSend: vi.fn(),
  chatStatus: vi.fn(),
  chatStop: vi.fn(),
  fetchModelCatalogue: vi.fn(),
  onChatComplete: vi.fn(),
  onChatFailed: vi.fn(),
  onChatToken: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({
  chatClose,
  chatDelete,
  chatList,
  chatLoad,
  chatOpenModel,
  chatSend,
  chatStatus,
  chatStop,
  fetchModelCatalogue,
  onChatComplete,
  onChatFailed,
  onChatToken,
}));

const MODEL_NAME = 'Mistral-7B-Q4_K_M';

/** What Rust says it settled on when a model is open. */
function session(): ModelSession {
  return {
    modelName: MODEL_NAME,
    gpuLayers: 32,
    contextLength: 8192,
    fits: true,
    headDimDerived: false,
    vision: false,
  };
}

/** The conversation the page resumes, so a message has somewhere to go. */
const STORED: Conversation = {
  id: 'c1',
  title: 'About tea',
  modelName: MODEL_NAME,
  messages: [],
};

/**
 * The refusal `chat_send` returns when nothing is open.
 *
 * Copied from `src-tauri/src/chat.rs`, because it is the sentence the user
 * reported seeing and the assertions below are about that exact experience
 * rather than about some error having happened.
 */
const NO_MODEL = 'no model is open; open one before sending a message';

/** The session Rust is holding, or `null` when it is holding none. */
let held: ModelSession | null = null;

/**
 * Waits long enough for a close to have run if anything was going to fire one.
 *
 * "The session was not closed" is a claim about something not happening, so it
 * is only worth asserting after enough time for it to have happened. Real time
 * rather than fake: the page awaits promises across this window, and fake timers
 * would freeze the half of the sequence that is not a timer.
 */
async function longEnoughForACloseToHaveFired(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 400));
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  held = null;

  chatOpenModel.mockImplementation(() => {
    held = session();
    return Promise.resolve(held);
  });
  chatClose.mockImplementation(() => {
    held = null;
    return Promise.resolve();
  });
  chatStatus.mockImplementation(() => Promise.resolve(held));
  chatSend.mockImplementation(() =>
    held === null ? Promise.reject(new Error(NO_MODEL)) : Promise.resolve()
  );
  chatStop.mockResolvedValue(undefined);
  chatList.mockResolvedValue([STORED]);
  chatLoad.mockResolvedValue(STORED);
  chatDelete.mockResolvedValue(undefined);
  fetchModelCatalogue.mockResolvedValue([]);
  onChatToken.mockImplementation((_handler: (payload: ChatToken) => void) =>
    Promise.resolve(() => undefined)
  );
  onChatComplete.mockImplementation((_handler: (payload: ChatComplete) => void) =>
    Promise.resolve(() => undefined)
  );
  onChatFailed.mockImplementation((_handler: (payload: ChatFailure) => void) =>
    Promise.resolve(() => undefined)
  );
});

/** What pressing Run on the LLM tab does before this page is ever rendered. */
async function runOpensAModel(): Promise<void> {
  await chatOpenModel(`C:\\models\\${MODEL_NAME}.gguf`);
  chatOpenModel.mockClear();
}

/** Types a message and presses send. */
async function sendAMessage(): Promise<void> {
  await userEvent.type(await screen.findByLabelText('Message'), 'hello');
  await userEvent.click(screen.getByRole('button', { name: /^send$/i }));
}

describe('the chat session across the page mounting', () => {
  it('sends a message when the model was opened before the page mounted', async () => {
    // The user's bug, end to end: press Run on the LLM tab, land here, type,
    // and be told no model is open. The session is real -- Run opened it -- so
    // a page that refuses to use it is refusing something it can see.
    await runOpensAModel();

    render(
      <StrictMode>
        <Chat opened={session()} />
      </StrictMode>
    );
    await screen.findAllByText(MODEL_NAME);

    await sendAMessage();

    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith(STORED.id, 'hello', null);
    });
    expect(screen.queryByText(new RegExp(NO_MODEL, 'i'))).not.toBeInTheDocument();
    expect(held).not.toBeNull();
  });

  it('survives a mount, an unmount and a mount again', async () => {
    // What StrictMode does to every effect in development, written out. It is
    // not a development-only concern: any remount is this sequence, and a
    // cleanup that closed unconditionally broke all of them.
    await runOpensAModel();

    const first = render(<Chat opened={session()} />);
    await screen.findAllByText(MODEL_NAME);
    first.unmount();
    render(<Chat opened={session()} />);
    await screen.findAllByText(MODEL_NAME);

    await longEnoughForACloseToHaveFired();

    expect(held).not.toBeNull();
    expect(chatClose).not.toHaveBeenCalled();

    // Usable, not merely present: the model bar is drawn from the page's own
    // state and would look identical over a session Rust had already dropped.
    await sendAMessage();
    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith(STORED.id, 'hello', null);
    });
    expect(screen.queryByText(new RegExp(NO_MODEL, 'i'))).not.toBeInTheDocument();
  });

  it('keeps the model loaded across navigating away and back', async () => {
    // The reversal, as the user described it: open a model, go and look at
    // something else, come back, and carry on the same conversation with the
    // same model without waiting for it to load again. Against the old code
    // this fails on the first assertion -- the unmount closed the session.
    await runOpensAModel();

    // Away.
    const chat = render(<Chat opened={session()} />);
    await screen.findAllByText(MODEL_NAME);
    chat.unmount();

    await longEnoughForACloseToHaveFired();

    expect(chatClose).not.toHaveBeenCalled();
    expect(held).not.toBeNull();

    // And back. The shell hands the session straight back, and Rust confirms
    // it, so there is no reload and no no-model form in between.
    render(<Chat opened={held} />);
    await screen.findAllByText(MODEL_NAME);

    expect(screen.queryByLabelText(/model file/i)).not.toBeInTheDocument();
    expect(chatOpenModel).not.toHaveBeenCalled();

    // The conversation is where it was, and usable.
    expect(await screen.findAllByText(STORED.title)).not.toHaveLength(0);
    await sendAMessage();
    await waitFor(() => {
      expect(chatSend).toHaveBeenCalledWith(STORED.id, 'hello', null);
    });
    expect(screen.queryByText(new RegExp(NO_MODEL, 'i'))).not.toBeInTheDocument();
  });

  it('still stops the session when the user asks it to', async () => {
    // The other half of the reversal, and the half that keeps osstat honest.
    // Nothing closes the model implicitly any more, so the explicit control is
    // now the only way a user can give several gigabytes back without quitting
    // -- if it stopped working the memory would have no way out at all.
    await runOpensAModel();

    render(<Chat opened={session()} />);
    await screen.findAllByText(MODEL_NAME);

    await userEvent.click(screen.getByRole('button', { name: /^unload$/i }));

    await waitFor(() => {
      expect(chatClose).toHaveBeenCalledTimes(1);
    });
    expect(held).toBeNull();
    expect(await screen.findByLabelText(/model file/i)).toBeInTheDocument();
  });

  it('tells the shell what is open, so another tab can say so', async () => {
    // What the marker on the Chat entry is drawn from. A model is now invisible
    // while the user is on another page, and the shell is the only thing still
    // mounted to remember it -- so this reporting is the whole of how a
    // multi-gigabyte allocation stays visible at all.
    await runOpensAModel();
    const reported: (ModelSession | null)[] = [];

    render(
      <Chat
        opened={session()}
        onSessionChange={(open) => {
          reported.push(open);
        }}
      />
    );
    await screen.findAllByText(MODEL_NAME);

    expect(reported.at(-1)).not.toBeNull();

    await userEvent.click(screen.getByRole('button', { name: /^unload$/i }));

    await waitFor(() => {
      expect(reported.at(-1)).toBeNull();
    });
  });

  it('shows the no-model form when nothing is open, whatever it was handed', async () => {
    // The stale half of the same problem. The shell can hand this page a
    // session that has since been closed -- by leaving and returning, or by a
    // server that stopped on its own -- and drawing a model bar for it puts
    // the user in front of a composer whose every message will be refused.
    render(<Chat opened={session()} />);

    expect(await screen.findByLabelText(/model file/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /open model/i })).toBeInTheDocument();
  });
});
