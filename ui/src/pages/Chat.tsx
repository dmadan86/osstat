/**
 * Chatting with a model this machine is running.
 *
 * Three parts, top to bottom: a model bar carrying the session's identity and
 * the context meter, the transcript, and a composer. The meter is the same
 * `Meter` the Overview draws CPU and RAM with, and that reuse is the point —
 * osstat is a monitoring application, and the context window is the resource
 * governing whether the model still remembers the start of the conversation.
 *
 * **Model output is plain text.** No markdown library, no
 * `dangerouslySetInnerHTML`, no HTML constructed from anything the model wrote —
 * so there is no sanitiser here to get wrong. Fenced code blocks are the one
 * structure detected, and they become `<pre><code>` holding a text node. The
 * cost is stated plainly in the design: a model that formats heavily will show
 * literal `**` and `|`. Accepted.
 *
 * The page issues no HTTP request of its own and never learns the server's port
 * or key. Everything below travels over the `chat_*` commands and the `chat:*`
 * events, which is ADR-012's stated security property rather than a style
 * preference.
 */

import { useEffect, useReducer, useState } from 'react';

import type { ChatComplete } from '../bindings/ChatComplete';
import type { ChatFailure } from '../bindings/ChatFailure';
import type { ChatToken } from '../bindings/ChatToken';
import type { Conversation } from '../bindings/Conversation';
import type { Message } from '../bindings/Message';
import type { ModelSession } from '../bindings/ModelSession';
import type { Role } from '../bindings/Role';
import type { Timings } from '../bindings/Timings';
import type { Usage } from '../bindings/Usage';
import { Meter } from '../components/Meter';
import { formatCount } from '../lib/format';
import {
  chatClose,
  chatList,
  chatOpenModel,
  chatSend,
  chatStop,
  onChatComplete,
  onChatFailed,
  onChatToken,
} from '../lib/ipc';

/** The reply currently streaming, which is not yet a stored message. */
interface Pending {
  /** Everything that has arrived so far. */
  content: string;
  /** Whether the user has asked for it to stop. */
  stopped: boolean;
}

/** The conversation on screen and whatever is happening to it. */
interface Transcript {
  /** The conversation being held, including every finished turn. */
  conversation: Conversation;
  /** The reply streaming right now, or `null` between replies. */
  pending: Pending | null;
  /** The latest speeds, shown only while a reply is streaming. */
  timings: Timings | null;
  /** Why the last reply could not be finished, if it could not. */
  failure: string | null;
}

/** Everything that can happen to a transcript. */
type Action =
  | { kind: 'open'; conversation: Conversation }
  | { kind: 'ask'; text: string }
  | { kind: 'token'; conversationId: string; delta: string; timings: Timings | null }
  | { kind: 'stopping' }
  | { kind: 'complete'; conversationId: string; usage: Usage | null; stopped: boolean }
  | { kind: 'failed'; conversationId: string; message: string };

/** How each role is introduced in the transcript. */
const ROLE_LABEL: Record<Role, string> = {
  system: 'System',
  user: 'You',
  assistant: 'Model',
};

/** What an unnamed conversation is called until its first message names it. */
const UNTITLED = 'New conversation';

/**
 * A fresh identifier for a conversation.
 *
 * Time-ordered so that the stored list — which the backend sorts by id — comes
 * back oldest first, and restricted to the characters `ConversationStore`
 * accepts in a file stem.
 */
function newConversationId(): string {
  const random = Math.random().toString(36).slice(2, 8);
  return `${Date.now().toString(36)}-${random}`;
}

/** A conversation with nothing in it yet. */
function freshConversation(modelName: string): Conversation {
  return { id: newConversationId(), title: UNTITLED, modelName, messages: [] };
}

/** What to call a conversation, from the message that started it. */
function titleOf(text: string): string {
  const trimmed = text.trim();
  const title = [...trimmed].slice(0, 60).join('');

  if (title === '') return UNTITLED;
  return title.length < trimmed.length ? `${title}…` : title;
}

/** Renders an unknown thrown value as a message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Applies one event to the transcript.
 *
 * A reducer rather than several `useState` calls because the pieces move
 * together: a completion has to read the text the tokens accumulated, and
 * reading one piece of state from inside another's setter is how a reply ends
 * up empty. The conversation identifier is checked here, at the one place every
 * event passes through.
 */
function reduce(state: Transcript, action: Action): Transcript {
  switch (action.kind) {
    case 'open':
      return { conversation: action.conversation, pending: null, timings: null, failure: null };

    case 'ask':
      return {
        conversation: {
          ...state.conversation,
          title:
            state.conversation.messages.length === 0
              ? titleOf(action.text)
              : state.conversation.title,
          messages: [
            ...state.conversation.messages,
            { role: 'user', content: action.text, usage: null, stopped: false },
          ],
        },
        pending: { content: '', stopped: false },
        timings: null,
        failure: null,
      };

    case 'token': {
      if (action.conversationId !== state.conversation.id) return state;

      return {
        ...state,
        // A token with no pending reply behind it still belongs to this
        // conversation: the events are what the transcript is made of, and
        // dropping one because the send was not seen would lose text.
        pending: {
          content: (state.pending?.content ?? '') + action.delta,
          stopped: state.pending?.stopped ?? false,
        },
        timings: action.timings ?? state.timings,
      };
    }

    case 'stopping':
      return state.pending === null
        ? state
        : { ...state, pending: { ...state.pending, stopped: true }, timings: null };

    case 'complete': {
      if (action.conversationId !== state.conversation.id) return state;

      const reply: Message = {
        role: 'assistant',
        content: state.pending?.content ?? '',
        usage: action.usage,
        stopped: action.stopped || (state.pending?.stopped ?? false),
      };

      return {
        conversation: {
          ...state.conversation,
          messages: [...state.conversation.messages, reply],
        },
        pending: null,
        timings: null,
        failure: null,
      };
    }

    case 'failed': {
      if (action.conversationId !== state.conversation.id) return state;

      // Whatever arrived before the failure is kept and marked incomplete.
      // Losing a half-written answer punishes the user for the model being too
      // big for the machine, which is not their mistake to pay for.
      const partial = state.pending?.content ?? '';
      const messages =
        partial === ''
          ? state.conversation.messages
          : [
              ...state.conversation.messages,
              { role: 'assistant' as const, content: partial, usage: null, stopped: true },
            ];

      return {
        conversation: { ...state.conversation, messages },
        pending: null,
        timings: null,
        failure: action.message,
      };
    }
  }
}

/**
 * How much of the context window the conversation occupies.
 *
 * Read from the most recent exchange rather than summed over the transcript:
 * `promptTokens` already counts everything the server was sent, so adding the
 * earlier turns would count them twice.
 */
function usedTokens(messages: readonly Message[]): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const usage = messages[index]?.usage;
    if (usage !== null && usage !== undefined) {
      return usage.promptTokens + usage.completionTokens;
    }
  }
  return 0;
}

/** One piece of a message: prose, or the inside of a fence. */
interface Segment {
  /** Whether this is code. */
  code: boolean;
  /** The text itself, with the fence and its language line removed. */
  body: string;
  /** The language the fence declared, where it declared one. */
  language: string;
}

/**
 * Splits model output on ``` fences.
 *
 * Deliberately the whole of the markdown handling. Even indices are prose and
 * odd indices are code, which also means a fence still being streamed renders
 * as code from the moment it opens rather than flickering into place when it
 * closes.
 */
function splitFences(text: string): Segment[] {
  return text.split('```').map((part, index) => {
    if (index % 2 === 0) return { code: false, body: part, language: '' };

    const newline = part.indexOf('\n');
    const first = newline === -1 ? '' : part.slice(0, newline);
    // A fence's first line is a language tag only if it looks like one; code
    // that starts on the fence line itself must not lose its first line.
    const language = /^[\w+#.-]*$/.test(first) ? first : '';
    const body = language === '' && newline !== -1 ? part : part.slice(newline + 1);

    return { code: true, body: body.replace(/\n$/, ''), language };
  });
}

/**
 * Renders model output as text.
 *
 * No HTML is constructed from any of it. Prose becomes a `<span>` that
 * preserves whitespace; a fence becomes a `<pre><code>` holding a text node and
 * a copy control beside it.
 */
function renderText(text: string): React.JSX.Element[] {
  return splitFences(text)
    .map((segment, index) => ({ segment, index }))
    .filter(({ segment }) => segment.body !== '')
    .map(({ segment, index }) =>
      segment.code ? (
        <CodeBlock key={index} language={segment.language} body={segment.body} />
      ) : (
        <span key={index} className="whitespace-pre-wrap break-words">
          {segment.body}
        </span>
      )
    );
}

/** One fenced block, with the language it declared and a copy control. */
function CodeBlock({ language, body }: { language: string; body: string }): React.JSX.Element {
  return (
    <div className="my-1 overflow-hidden rounded-lg border border-edge bg-black/30">
      <div className="flex items-center justify-between gap-3 border-b border-edge px-2 py-0.5">
        <span className="font-mono text-[10px] uppercase tracking-wider text-neutral-500">
          {language === '' ? 'code' : language}
        </span>
        <button
          type="button"
          onClick={() => {
            void navigator.clipboard?.writeText(body).catch(() => {
              // A clipboard the platform refuses is not worth an error banner
              // over a block the user can still select.
            });
          }}
          className="rounded-md border border-edge px-1.5 text-[10px] text-neutral-400 hover:bg-white/[0.04]"
        >
          Copy
        </button>
      </div>
      <pre className="overflow-x-auto px-2 py-1.5 text-xs">
        <code data-selectable className="font-mono">
          {body}
        </code>
      </pre>
    </div>
  );
}

/** What the chat page needs from the shell. */
export interface ChatProps {
  /**
   * A session the advisor's Run control already opened, or `null`.
   *
   * The model is opened by whoever pressed Run, through the same
   * `chat_open_model` the file picker calls, so this page receives a session
   * rather than a second path to open. The shell drops it when the user
   * navigates away, because leaving this page ends the server.
   */
  opened?: ModelSession | null;
}

/**
 * Renders the chat page.
 *
 * @param props A session opened elsewhere, if there is one.
 */
export function Chat({ opened = null }: ChatProps = {}): React.JSX.Element {
  const [state, dispatch] = useReducer(reduce, null, () => ({
    conversation: freshConversation(''),
    pending: null,
    timings: null,
    failure: null,
  }));
  const [session, setSession] = useState<ModelSession | null>(opened);
  const [path, setPath] = useState('');
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [draft, setDraft] = useState('');

  // Resume the most recent conversation. The store sorts by identifier and
  // identifiers are time-ordered, so the last entry is the newest.
  useEffect(() => {
    let cancelled = false;

    chatList().then(
      (found) => {
        const latest = found.at(-1);
        if (!cancelled && latest !== undefined) dispatch({ kind: 'open', conversation: latest });
      },
      () => {
        // A store that cannot be listed is no reason to refuse a new
        // conversation; the page starts empty instead.
      }
    );

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const unlisten = onChatToken((payload: ChatToken) => {
      dispatch({
        kind: 'token',
        conversationId: payload.conversationId,
        delta: payload.delta,
        timings: payload.timings,
      });
    });

    return () => {
      unlisten.then(
        (off) => {
          off();
        },
        () => {
          // Nothing to unsubscribe from if the subscription itself failed.
        }
      );
    };
  }, []);

  useEffect(() => {
    const unlisten = onChatComplete((payload: ChatComplete) => {
      dispatch({
        kind: 'complete',
        conversationId: payload.conversationId,
        usage: payload.usage,
        stopped: payload.stopped,
      });
    });

    return () => {
      unlisten.then(
        (off) => {
          off();
        },
        () => {
          // Nothing to unsubscribe from if the subscription itself failed.
        }
      );
    };
  }, []);

  useEffect(() => {
    const unlisten = onChatFailed((payload: ChatFailure) => {
      dispatch({
        kind: 'failed',
        conversationId: payload.conversationId,
        message: payload.message,
      });
    });

    return () => {
      unlisten.then(
        (off) => {
          off();
        },
        () => {
          // Nothing to unsubscribe from if the subscription itself failed.
        }
      );
    };
  }, []);

  // Leaving the page ends the server. A loaded model holds several gigabytes of
  // VRAM; osstat itself holds a fraction of that, and a monitoring application
  // that quietly kept the larger allocation alive would be the worst neighbour
  // on the machine.
  useEffect(
    () => () => {
      chatClose().catch(() => {
        // Nothing useful to say to a page that is already gone.
      });
    },
    []
  );

  const streaming = state.pending !== null;

  function open(): void {
    if (path.trim() === '' || opening) return;

    setOpening(true);
    setOpenError(null);
    chatOpenModel(path.trim()).then(
      (opened) => {
        setSession(opened);
        setOpening(false);
      },
      (error: unknown) => {
        setOpenError(messageOf(error));
        setOpening(false);
      }
    );
  }

  function send(): void {
    const text = draft.trim();
    if (text === '' || session === null || streaming) return;

    const id = state.conversation.id;
    setDraft('');
    dispatch({ kind: 'ask', text });
    chatSend(id, text).catch((error: unknown) => {
      dispatch({ kind: 'failed', conversationId: id, message: messageOf(error) });
    });
  }

  function stop(): void {
    dispatch({ kind: 'stopping' });
    chatStop().catch(() => {
      // A reply that has already finished has nothing left to stop.
    });
  }

  return (
    <div className="flex h-full flex-col gap-3">
      {session === null ? (
        <OpenModel
          path={path}
          opening={opening}
          error={openError}
          onPathChange={setPath}
          onOpen={open}
        />
      ) : (
        <ModelBar
          session={session}
          title={state.conversation.title}
          used={usedTokens(state.conversation.messages)}
          streaming={streaming}
          failure={state.failure}
          onStop={stop}
          onNew={() => {
            dispatch({ kind: 'open', conversation: freshConversation(session.modelName) });
          }}
        />
      )}

      <div className="min-h-0 flex-1 overflow-auto rounded-xl border border-edge bg-surface-raised p-3">
        {state.conversation.messages.length === 0 && state.pending === null && (
          <p className="text-center text-sm text-neutral-500">
            {session === null
              ? 'Open a GGUF model file to start a conversation.'
              : 'Nothing said yet.'}
          </p>
        )}

        <ol className="flex flex-col gap-3">
          {state.conversation.messages.map((message, index) => (
            <Turn key={index} message={message} />
          ))}

          {state.pending !== null && (
            <Turn
              message={{
                role: 'assistant',
                content: state.pending.content,
                usage: null,
                stopped: state.pending.stopped,
              }}
              timings={state.pending.stopped ? null : state.timings}
            />
          )}
        </ol>
      </div>

      {session !== null && (
        <Composer draft={draft} streaming={streaming} onDraftChange={setDraft} onSend={send} />
      )}
    </div>
  );
}

/** The control that opens a model file. */
function OpenModel({
  path,
  opening,
  error,
  onPathChange,
  onOpen,
}: {
  path: string;
  opening: boolean;
  error: string | null;
  onPathChange: (value: string) => void;
  onOpen: () => void;
}): React.JSX.Element {
  return (
    <section
      aria-label="Open a model"
      className="shrink-0 rounded-xl border border-edge bg-surface-raised p-3"
    >
      <div className="flex flex-wrap items-end gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <label
            htmlFor="chat-model-path"
            className="text-[10px] uppercase tracking-wider text-neutral-500"
          >
            Model file
          </label>
          <input
            id="chat-model-path"
            type="text"
            value={path}
            spellCheck={false}
            placeholder="C:\models\Mistral-7B-Q4_K_M.gguf"
            onChange={(event) => {
              onPathChange(event.target.value);
            }}
            className="w-full rounded-md border border-edge bg-transparent px-2 py-1 font-mono text-xs"
          />
        </div>
        <button
          type="button"
          disabled={opening || path.trim() === ''}
          onClick={onOpen}
          className="rounded-md border border-accent px-3 py-1 text-xs text-accent transition-colors hover:bg-accent/10 disabled:opacity-40"
        >
          {opening ? 'Loading the model…' : 'Open model'}
        </button>
      </div>

      {error !== null && (
        <p role="alert" className="mt-2 text-xs text-red-400">
          {error}
        </p>
      )}

      <p className="mt-2 text-[11px] text-neutral-600">
        The layer count and context window are chosen from the file&rsquo;s own header and the
        measured VRAM, not from an estimate for a model of its name.
      </p>
    </section>
  );
}

/** The session's identity, its context meter, and the stop control. */
function ModelBar({
  session,
  title,
  used,
  streaming,
  failure,
  onStop,
  onNew,
}: {
  session: ModelSession;
  title: string;
  used: number;
  streaming: boolean;
  failure: string | null;
  onStop: () => void;
  onNew: () => void;
}): React.JSX.Element {
  return (
    <section
      aria-label="Session"
      className="shrink-0 rounded-xl border border-edge bg-surface-raised p-3"
    >
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-1">
        <span data-selectable className="font-mono text-sm">
          {session.modelName}
        </span>
        <span className="font-mono text-xs text-neutral-500">
          {String(session.gpuLayers)} layers on GPU
        </span>
        <span className="truncate text-xs text-neutral-500">{title}</span>

        <div className="ml-auto flex items-center gap-2">
          {streaming && (
            <button
              type="button"
              onClick={onStop}
              className="rounded-md border border-edge px-2 py-0.5 text-xs text-neutral-300 hover:bg-white/[0.04]"
            >
              Stop
            </button>
          )}
          <button
            type="button"
            onClick={onNew}
            className="rounded-md border border-edge px-2 py-0.5 text-xs text-neutral-400 hover:bg-white/[0.04]"
          >
            New conversation
          </button>
        </div>
      </div>

      <div className="mt-2">
        <Meter
          label="Context"
          fraction={used / session.contextLength}
          detail={`${formatCount(used)} of ${formatCount(session.contextLength)} tokens`}
          warnWhenFull
        />
      </div>

      {failure !== null && (
        <p role="alert" className="mt-2 border-t border-edge pt-2 text-xs text-red-400">
          {failure}
        </p>
      )}

      {!session.fits && (
        <p className="mt-2 text-[11px] text-amber-400/80">
          Not every layer fitted in VRAM, so the rest run on the CPU and generation will be slower.
          The figure is an estimate, which is why this is a warning rather than a refusal.
        </p>
      )}

      {session.headDimDerived && (
        <p className="mt-1 text-[11px] text-neutral-600">
          This model&rsquo;s header declares no attention key length, so the KV-cache arithmetic
          derived one. That is correct for standard attention and wrong for models that diverge.
        </p>
      )}
    </section>
  );
}

/** One turn of the conversation. */
function Turn({
  message,
  timings = null,
}: {
  message: Message;
  timings?: Timings | null;
}): React.JSX.Element {
  const speeds = timings;

  return (
    <li className="flex flex-col gap-1">
      <span className="text-[10px] uppercase tracking-wider text-neutral-500">
        {ROLE_LABEL[message.role]}
      </span>

      <div
        data-selectable
        className={`text-sm ${message.role === 'user' ? 'text-neutral-200' : 'text-neutral-300'}`}
      >
        {renderText(message.content)}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        {message.usage !== null && (
          <span data-selectable className="font-mono text-[11px] text-neutral-600">
            {`${formatCount(message.usage.promptTokens)} in · ${formatCount(
              message.usage.completionTokens
            )} out`}
          </span>
        )}

        {message.stopped && <span className="text-[11px] text-amber-400/80">Stopped</span>}

        {speeds !== null && speeds.promptPerSecond !== null && (
          <span className="font-mono text-[11px] text-neutral-600">
            {`${speeds.promptPerSecond.toFixed(1)} tok/s prompt`}
          </span>
        )}

        {speeds !== null && speeds.predictedPerSecond !== null && (
          <span className="font-mono text-[11px] text-neutral-400">
            {`${speeds.predictedPerSecond.toFixed(1)} tok/s`}
          </span>
        )}
      </div>
    </li>
  );
}

/** The message box and its send control. */
function Composer({
  draft,
  streaming,
  onDraftChange,
  onSend,
}: {
  draft: string;
  streaming: boolean;
  onDraftChange: (value: string) => void;
  onSend: () => void;
}): React.JSX.Element {
  return (
    <form
      className="flex shrink-0 items-end gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        onSend();
      }}
    >
      <label htmlFor="chat-message" className="sr-only">
        Message
      </label>
      <textarea
        id="chat-message"
        rows={2}
        value={draft}
        disabled={streaming}
        placeholder={streaming ? 'Waiting for the reply…' : 'Say something'}
        onChange={(event) => {
          onDraftChange(event.target.value);
        }}
        onKeyDown={(event) => {
          // Enter sends, Shift+Enter adds a line. The composer is two rows
          // tall, so the multi-line case has to stay reachable.
          if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault();
            onSend();
          }
        }}
        className="min-w-0 flex-1 resize-none rounded-md border border-edge bg-transparent px-2 py-1 text-sm disabled:opacity-50"
      />
      <button
        type="submit"
        disabled={streaming || draft.trim() === ''}
        className="rounded-md border border-accent px-3 py-1.5 text-xs text-accent transition-colors hover:bg-accent/10 disabled:opacity-40"
      >
        Send
      </button>
    </form>
  );
}
