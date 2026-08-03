/**
 * Chatting with a model this machine is running.
 *
 * Three parts, top to bottom: a model bar carrying the session's identity, the
 * chooser that swaps one model for another, and the context meter; then the
 * transcript; then a composer. The meter is the same
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

import { useCallback, useEffect, useReducer, useRef, useState } from 'react';

import type { ChatComplete } from '../bindings/ChatComplete';
import type { ChatFailure } from '../bindings/ChatFailure';
import type { ChatToken } from '../bindings/ChatToken';
import type { Conversation } from '../bindings/Conversation';
import type { Message } from '../bindings/Message';
import type { ModelCatalogueEntry } from '../bindings/ModelCatalogueEntry';
import type { ModelSession } from '../bindings/ModelSession';
import type { Provenance } from '../bindings/Provenance';
import type { Role } from '../bindings/Role';
import type { Timings } from '../bindings/Timings';
import type { Usage } from '../bindings/Usage';
import { Meter } from '../components/Meter';
import { formatCount, formatTimeOfDay } from '../lib/format';
import {
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
} from '../lib/ipc';
import { UNREVIEWED } from '../lib/provenance';

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
  | {
      kind: 'complete';
      conversationId: string;
      usage: Usage | null;
      stopped: boolean;
      elapsedSeconds: number;
    }
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

/** One downloaded model the header can switch to. */
interface Downloaded {
  /**
   * What the model will be called once it is open.
   *
   * Derived the same way Rust derives it, so the name in the dropdown and the
   * name in the model bar are the same string rather than two spellings of the
   * same file that happen to agree on the models anyone has tried.
   */
  name: string;
  /**
   * The record's own absolute path, which is what `chat_open_model` is given.
   *
   * Nothing here reconstructs a path from a folder and a file name — the same
   * rule the LLM tab's Run control follows.
   */
  path: string;
  /** Which verification tier fetched it. */
  provenance: Provenance;
}

/**
 * A model file's name, the way `chat_open_model` reports it back.
 *
 * The Rust side names a session from the file stem, so this drops the directory
 * and the extension to match. Both separators are handled because a Windows
 * path is what this application mostly sees and a POSIX one is what its tests
 * and its Linux builds mostly see.
 */
function stemOf(path: string): string {
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1);
  const dot = name.lastIndexOf('.');

  // `dot <= 0` rather than `=== -1`: a name that is nothing but an extension
  // keeps it, which is what `Path::file_stem` does with the same input.
  return dot <= 0 ? name : name.slice(0, dot);
}

/**
 * The catalogue entries that can actually be opened.
 *
 * Downloaded *and* holding a path: a model that is not on disk cannot be
 * loaded, so offering it would be offering a control whose only outcome is an
 * error. The `path === null` half is not defensive — the catalogue carries
 * pinned cells nobody has fetched, and those are exactly the rows with no file
 * behind them.
 */
function downloadedFrom(entries: readonly ModelCatalogueEntry[]): Downloaded[] {
  return entries.flatMap((entry) =>
    entry.state === 'downloaded' && entry.path !== null
      ? [{ name: stemOf(entry.path), path: entry.path, provenance: entry.provenance }]
      : []
  );
}

/**
 * How one model reads in the dropdown.
 *
 * A searched model carries the unreviewed marker here exactly as it does in the
 * LLM tab's list. The two tiers are a security distinction, not a detail of the
 * page that happened to introduce them, so a model must not become
 * indistinguishable from a reviewed one by being reached through a different
 * control.
 */
function labelOf(model: Downloaded): string {
  return model.provenance === 'searched' ? `${model.name} — ${UNREVIEWED}` : model.name;
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
            {
              role: 'user',
              content: action.text,
              usage: null,
              stopped: false,
              // A question takes as long as the person typing it, which is not
              // a generation time and not osstat's to measure.
              elapsedSeconds: null,
              // Stamped here as well as in the backend, which saves its own.
              // This copy is what the user sees the moment they press Enter;
              // the stored one replaces it when the conversation is reopened,
              // and the two are the same wall clock a fraction of a second
              // apart.
              sentAt: Date.now(),
            },
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
        elapsedSeconds: action.elapsedSeconds,
        sentAt: Date.now(),
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
              {
                role: 'assistant' as const,
                content: partial,
                usage: null,
                stopped: true,
                // The backend measured this turn and saved it with a time, but
                // a failure carries no figures with it. The stored message has
                // one; this on-screen copy of it does not, and inventing a
                // number to fill the gap would be worse than the gap.
                elapsedSeconds: null,
                sentAt: Date.now(),
              },
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
        <span className="font-mono text-[10px] uppercase tracking-wider text-text-muted">
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
          className="rounded-md border border-edge px-1.5 text-[10px] text-text-muted hover:bg-white/[0.04]"
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

/**
 * How close to the bottom still counts as being at the bottom, in pixels.
 *
 * Not zero: a transcript whose last line is a partly-drawn row of text sits a
 * fraction of a pixel short of its own height, and sub-pixel rounding differs
 * between platforms. A reader who has scrolled up to read is hundreds of pixels
 * away, not four, so nothing ambiguous falls in this band.
 */
const BOTTOM_SLACK = 4;

/**
 * Keeps a scrolling element pinned to its bottom, unless the user scrolled up.
 *
 * The rule that matters is the second half. Tokens arrive several times a
 * second, and a transcript that scrolls on every one of them makes reading
 * anything but the last line impossible — the reader scrolls up, the next token
 * yanks them back, and the only way to read the answer is to wait for it to
 * finish. So scrolling up switches the pinning off, and returning to the bottom
 * switches it back on. The user's scroll position is the input; nothing else
 * overrides it.
 *
 * `dependency` is whatever changes when there is new content: passing the
 * streamed text means every token is considered, and considering is cheap
 * because a detached reader does nothing at all.
 *
 * @param dependency A value that changes whenever the content grows.
 * @returns A ref to attach to the scrolling element.
 */
function useStickyScroll(dependency: unknown): React.RefObject<HTMLDivElement | null> {
  const ref = useRef<HTMLDivElement | null>(null);
  // A ref rather than state: this is read inside a scroll handler and written
  // on every wheel tick, and re-rendering the transcript to record where it is
  // scrolled to would be a redraw per frame for nothing visible.
  const stuck = useRef(true);

  useEffect(() => {
    const element = ref.current;
    if (element === null) return;

    function onScroll(): void {
      if (element === null) return;
      const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
      stuck.current = distance <= BOTTOM_SLACK;
    }

    element.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      element.removeEventListener('scroll', onScroll);
    };
  }, []);

  useEffect(() => {
    const element = ref.current;
    // The whole of the "does not fight the user" rule: when the reader has
    // scrolled up, new content changes nothing about where they are looking.
    if (element === null || !stuck.current) return;

    element.scrollTop = element.scrollHeight;
  }, [dependency]);

  return ref;
}

/**
 * How long a close waits after this page unmounts, in milliseconds.
 *
 * Long enough that a page which is coming straight back cancels it, short
 * enough that a user who has genuinely left does not notice the model outliving
 * their leaving. Nothing in the app schedules work in this window, so the only
 * thing that ever lands inside it is a remount.
 */
const CLOSE_GRACE_MS = 150;

/**
 * The close a previous unmount scheduled, or `null` when none is pending.
 *
 * Module scope rather than a ref, and deliberately so: what this records is
 * "no chat page is mounted", which is a fact about the page and not about one
 * mounting of it. A ref would be replaced by the very remount that needs to
 * cancel the close — which is the case that matters, since a fresh instance is
 * exactly what a return to this page produces.
 */
let pendingClose: ReturnType<typeof setTimeout> | null = null;

/** Calls off a close a previous unmount scheduled, if one is still pending. */
function keepSessionOpen(): void {
  if (pendingClose === null) return;

  clearTimeout(pendingClose);
  pendingClose = null;
}

/**
 * Schedules the close that leaving this page performs.
 *
 * Deferred rather than immediate, which is the whole of the fix. Unmounting is
 * not the same event as leaving: React's development double-invoke unmounts and
 * mounts again on purpose, and returning to the page does the same across a
 * navigation. A close fired straight from the cleanup ended the server in both
 * of those, and the page that came back could still see a session it no longer
 * had — the model bar was drawn and every message was refused with "no model is
 * open". Waiting a moment lets a remount say "still here" (ADR-013 keeps its
 * property: nobody who actually leaves keeps a multi-gigabyte allocation).
 */
function closeSessionShortly(): void {
  keepSessionOpen();

  pendingClose = setTimeout(() => {
    pendingClose = null;
    chatClose().catch(() => {
      // Nothing useful to say to a page that is already gone.
    });
  }, CLOSE_GRACE_MS);
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
   *
   * A head start rather than the truth: `chat_status` is asked on mount and its
   * answer wins. Rust owns the session, and a prop cannot know that the page
   * was mounted a second time or that the server has since stopped.
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
  const [conversations, setConversations] = useState<readonly Conversation[]>([]);
  const [models, setModels] = useState<readonly Downloaded[]>([]);
  /** The model a switch is loading, or `null` when none is. */
  const [switchingTo, setSwitchingTo] = useState<string | null>(null);
  // Whether this page has opened or closed a model itself since it mounted.
  // The adoption below is a question asked on mount and answered a tick later,
  // and an answer that arrived after the user had already acted would undo what
  // they did. A ref rather than state because nothing renders differently for
  // it -- it only decides whether a late answer is still worth applying.
  const decided = useRef(false);

  // Re-reads the stored list. Called wherever the store has just changed --
  // after a message is accepted, after a reply is saved, after a delete --
  // because `chat_list` reads the directory and the directory is the truth.
  const refresh = useCallback(() => {
    chatList().then(setConversations, () => {
      // A store that cannot be listed is no reason to refuse a conversation;
      // the list stays as it was rather than emptying itself on a stumble.
    });
  }, []);

  // Adopt whatever session Rust already has, rather than assuming this page's
  // own lifetime opened it. It usually did not: pressing Run on the LLM tab
  // opens the model and *then* comes here, and the session survives this page
  // being unmounted and mounted again. The `opened` prop is only a head start
  // on the same answer -- it saves a frame of the no-model form, and it can be
  // stale, so Rust settles it either way.
  useEffect(() => {
    let cancelled = false;

    chatStatus().then(
      (open) => {
        if (cancelled || decided.current) return;
        setSession(open);
      },
      () => {
        // A status that cannot be read leaves the page showing whatever the
        // shell handed it. Refusing to draw a session Rust may well have open
        // would be a worse guess than the one the prop already made.
      }
    );

    return () => {
      cancelled = true;
    };
  }, []);

  // Resume the most recent conversation. The store sorts by identifier and
  // identifiers are time-ordered, so the last entry is the newest.
  useEffect(() => {
    let cancelled = false;

    chatList().then(
      (found) => {
        if (cancelled) return;
        setConversations(found);
        const latest = found.at(-1);
        if (latest !== undefined) dispatch({ kind: 'open', conversation: latest });
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

  // What the header's dropdown offers. Read once on mount, which is enough:
  // the shell unmounts this page when the user leaves it, so a model downloaded
  // on the LLM tab is picked up by the remount on the way back.
  useEffect(() => {
    let cancelled = false;

    fetchModelCatalogue().then(
      (entries) => {
        if (cancelled) return;
        setModels(downloadedFrom(entries));
      },
      () => {
        // A catalogue that cannot be read leaves the header saying nothing is
        // downloaded. That is the wrong reason for the right advice — the LLM
        // tab is where both a missing library and a broken index are visible —
        // and it is better than a dropdown listing models it cannot open.
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
        elapsedSeconds: payload.elapsedSeconds,
      });
      // The reply has been written by the time this arrives, so the list is
      // one entry or one title out of date until it is read again.
      refresh();
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
  }, [refresh]);

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
  // on the machine. Mounting cancels the close the last unmount scheduled,
  // which is what tells a remount apart from a departure -- see
  // `closeSessionShortly`.
  useEffect(() => {
    keepSessionOpen();
    return closeSessionShortly;
  }, []);

  const streaming = state.pending !== null;
  const switching = switchingTo !== null;
  // Keyed on the streamed text and the turn count, which between them change
  // on every token and on every finished turn -- the two moments the transcript
  // grows. Switching conversations changes the count too, so opening one lands
  // at its end rather than at whatever offset the last one was left at.
  const transcriptRef = useStickyScroll(
    `${state.conversation.id}:${String(state.conversation.messages.length)}:${
      state.pending?.content ?? ''
    }`
  );

  function open(): void {
    if (path.trim() === '' || opening) return;

    decided.current = true;
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

  /**
   * Swaps the open model for another downloaded one.
   *
   * **`chat_close` then `chat_open_model`, in that order, and the second only
   * once the first has resolved.** Two servers alive at once is not a cosmetic
   * problem: each holds every byte of its weights, so a 7B model left running
   * beside the one being loaded is several gigabytes of VRAM held for a model
   * nobody is talking to — on the machine that is about to need it. Firing both
   * commands together would make the overlap the normal case rather than a
   * failure mode.
   *
   * A close that *fails* therefore opens nothing. Rust clears its own session
   * either way, so a refusal here means a child it can no longer reach; adding
   * a second one beside it would double the leak rather than recover from it.
   * The page drops to its no-model state, which is what Rust is now in.
   *
   * Reuses the two commands rather than adding a third that does both. One path
   * into a session means one place that reads the GGUF header, does the launch
   * arithmetic and builds the lockdown argument vector — a switch that skipped
   * any of it would be a session with different properties from every other.
   */
  async function switchModel(target: string): Promise<void> {
    if (target === '' || switching || streaming) return;

    decided.current = true;
    setSwitchingTo(stemOf(target));
    setOpenError(null);

    try {
      await chatClose();
    } catch (error: unknown) {
      setSession(null);
      setSwitchingTo(null);
      setOpenError(messageOf(error));
      return;
    }

    try {
      setSession(await chatOpenModel(target));
    } catch (error: unknown) {
      // The previous model is already closed by this point, so there is no
      // session left to fall back to and saying so is the honest state.
      setSession(null);
      setOpenError(messageOf(error));
    } finally {
      setSwitchingTo(null);
    }
  }

  function send(): void {
    const text = draft.trim();
    if (text === '' || session === null || streaming || switching) return;

    const id = state.conversation.id;
    setDraft('');
    dispatch({ kind: 'ask', text });
    chatSend(id, text).then(
      // The question is saved before the reply is asked for, so by the time
      // this resolves a conversation new to the store is on disk with its
      // title. Reading the list here is what puts it on screen.
      refresh,
      (error: unknown) => {
        dispatch({ kind: 'failed', conversationId: id, message: messageOf(error) });
      }
    );
  }

  /** Opens a stored conversation in place of the one on screen. */
  function select(id: string): void {
    if (id === state.conversation.id) return;

    chatLoad(id).then(
      (conversation) => {
        dispatch({ kind: 'open', conversation });
      },
      (error: unknown) => {
        // Surfaced rather than swallowed: a conversation that is listed but
        // will not open is a file the user can see and cannot reach.
        setOpenError(messageOf(error));
      }
    );
  }

  /** Deletes a stored conversation, and the transcript with it if it is open. */
  function remove(id: string): void {
    chatDelete(id).then(
      () => {
        refresh();
        if (id === state.conversation.id) {
          dispatch({ kind: 'open', conversation: freshConversation(session?.modelName ?? '') });
        }
      },
      (error: unknown) => {
        setOpenError(messageOf(error));
      }
    );
  }

  function stop(): void {
    dispatch({ kind: 'stopping' });
    chatStop().catch(() => {
      // A reply that has already finished has nothing left to stop.
    });
  }

  // Unloading ends the server and returns the page to its no-model state.
  // Navigating away already does this (ADR-013), but a 7B model holds around
  // five gigabytes resident and needing to leave the page to give that back is
  // not a way to ask for it. A reply still streaming ends as a failure, because
  // the stream it was reading no longer has a server behind it — which is the
  // truth, and better than a transcript waiting forever on a dead process.
  function unload(): void {
    if (switching) return;

    decided.current = true;
    setSession(null);
    setOpenError(null);
    chatClose().catch((error: unknown) => {
      setOpenError(messageOf(error));
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
          models={models}
          switchingTo={switchingTo}
          failure={state.failure}
          onStop={stop}
          onSwitch={(target) => {
            void switchModel(target);
          }}
          onUnload={unload}
          onNew={() => {
            dispatch({ kind: 'open', conversation: freshConversation(session.modelName) });
          }}
        />
      )}

      <div className="flex min-h-0 flex-1 gap-3">
        <ConversationList
          conversations={conversations}
          openId={state.conversation.id}
          onSelect={select}
          onDelete={remove}
        />

        {/* `log` rather than a bare div: a transcript that grows as tokens
            arrive is what the role describes, and it gives a screen reader the
            polite live region this page otherwise announces nothing through. */}
        <div
          ref={transcriptRef}
          role="log"
          aria-label="Transcript"
          className="min-h-0 flex-1 overflow-auto rounded-xl border border-edge bg-surface-raised p-3"
        >
          {state.conversation.messages.length === 0 && state.pending === null && (
            <EmptyTranscript hasModel={session !== null} hasStored={conversations.length > 0} />
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
                  // A reply still arriving has no elapsed time yet; the live
                  // tokens-per-second beside it is what says it is moving.
                  elapsedSeconds: null,
                  // Nor a time. It is stamped when it lands, so the label does
                  // not sit there naming a minute the reply has not finished
                  // in yet.
                  sentAt: null,
                }}
                timings={state.pending.stopped ? null : state.timings}
              />
            )}
          </ol>
        </div>
      </div>

      {session !== null && (
        <Composer
          draft={draft}
          streaming={streaming}
          switching={switching}
          onDraftChange={setDraft}
          onSend={send}
        />
      )}
    </div>
  );
}

/**
 * What an empty transcript says instead of nothing.
 *
 * A blank pane is indistinguishable from a page that failed to load, so this
 * says which of the two states the page is actually in and what the next action
 * is. The three cases are genuinely different next actions -- open a model,
 * pick up an old conversation, or type -- and collapsing them into one sentence
 * would send two thirds of readers to the wrong control.
 */
function EmptyTranscript({
  hasModel,
  hasStored,
}: {
  hasModel: boolean;
  hasStored: boolean;
}): React.JSX.Element {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
      <p className="text-sm text-text">{hasModel ? 'Nothing said yet' : 'No model is open'}</p>

      <p className="max-w-sm text-xs text-text-muted">
        {hasModel
          ? 'Type below and press Enter to send. Shift+Enter starts a new line.'
          : 'Open a GGUF model file above. Nothing is downloaded and nothing leaves this machine — the model runs as a local server osstat starts and stops with this page.'}
      </p>

      {hasStored && (
        <p className="text-xs text-text-muted">
          Or pick one of the saved conversations on the left to read it again.
        </p>
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
            className="text-[10px] uppercase tracking-wider text-text-muted"
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

      <p className="mt-2 text-[11px] text-text-faint">
        The layer count and context window are chosen from the file&rsquo;s own header and the
        measured VRAM, not from an estimate for a model of its name.
      </p>
    </section>
  );
}

/**
 * Every conversation on disk, with the open one marked.
 *
 * Conversations were being saved and listed by the backend already, and none
 * of it was reachable — so from the user's side nothing was kept at all. Each
 * entry names the model it was held with, because a conversation is only as
 * reproducible as the weights that answered it, and a transcript from a 3B
 * model reads very differently from the same questions put to a 70B one.
 */
function ConversationList({
  conversations,
  openId,
  onSelect,
  onDelete,
}: {
  conversations: readonly Conversation[];
  openId: string;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}): React.JSX.Element {
  return (
    <section
      aria-label="Conversations"
      className="flex w-56 shrink-0 flex-col overflow-auto rounded-xl border border-edge bg-surface-raised p-2"
    >
      <h2 className="px-1 pb-1 text-[10px] uppercase tracking-wider text-text-muted">
        Conversations
      </h2>

      {conversations.length === 0 ? (
        <p className="px-1 text-[11px] text-text-faint">Nothing saved yet.</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {conversations.map((conversation) => (
            <li key={conversation.id} className="flex items-center gap-1">
              <button
                type="button"
                aria-current={conversation.id === openId}
                onClick={() => {
                  onSelect(conversation.id);
                }}
                className={`min-w-0 flex-1 rounded-md px-1.5 py-1 text-left hover:bg-white/[0.04] ${
                  conversation.id === openId ? 'bg-white/[0.06]' : ''
                }`}
              >
                <span className="block truncate text-xs text-text">{conversation.title}</span>
                <span className="block truncate font-mono text-[10px] text-text-muted">
                  {conversation.modelName === '' ? 'no model recorded' : conversation.modelName}
                </span>
              </button>
              <button
                type="button"
                aria-label={`Delete ${conversation.title}`}
                onClick={() => {
                  onDelete(conversation.id);
                }}
                className="shrink-0 rounded-md border border-edge px-1.5 text-[10px] text-text-muted hover:bg-white/[0.04] hover:text-red-400"
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** The session's identity, its model chooser, its context meter and controls. */
function ModelBar({
  session,
  title,
  used,
  streaming,
  models,
  switchingTo,
  failure,
  onStop,
  onSwitch,
  onUnload,
  onNew,
}: {
  session: ModelSession;
  title: string;
  used: number;
  streaming: boolean;
  /** Every downloaded model, which is everything the chooser may offer. */
  models: readonly Downloaded[];
  /** The model a switch is loading, or `null` when none is. */
  switchingTo: string | null;
  failure: string | null;
  onStop: () => void;
  onSwitch: (path: string) => void;
  onUnload: () => void;
  onNew: () => void;
}): React.JSX.Element {
  const switching = switchingTo !== null;

  return (
    <section
      aria-label="Session"
      className="shrink-0 rounded-xl border border-edge bg-surface-raised p-3"
    >
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-1">
        {/* The model being loaded, not the one that was: `chat_close` has
            already run by the time this reads a name, so the previous one is
            gone and printing it would name weights nothing is holding. */}
        <span data-selectable className="font-mono text-sm">
          {switchingTo ?? session.modelName}
        </span>

        {/* The layer count belongs to the open session, so while a different
            model is loading there is no honest figure to show. Absent rather
            than stale: the new model may offload a different number, and a
            count carried over from the old one would be a measurement of
            something that is no longer running. */}
        {!switching && (
          <span className="font-mono text-xs text-text-muted">
            {String(session.gpuLayers)} layers on GPU
          </span>
        )}
        <span className="truncate text-xs text-text-muted">{title}</span>

        <div className="ml-auto flex items-center gap-2">
          {streaming && (
            <button
              type="button"
              onClick={onStop}
              className="rounded-md border border-edge px-2 py-0.5 text-xs text-text hover:bg-white/[0.04]"
            >
              Stop
            </button>
          )}
          <button
            type="button"
            onClick={onNew}
            className="rounded-md border border-edge px-2 py-0.5 text-xs text-text-muted hover:bg-white/[0.04]"
          >
            New conversation
          </button>
          <button
            type="button"
            disabled={switching}
            onClick={onUnload}
            title="End the server and give back the memory the weights hold"
            className="rounded-md border border-edge px-2 py-0.5 text-xs text-text-muted hover:bg-white/[0.04] disabled:opacity-40"
          >
            Unload
          </button>
        </div>
      </div>

      <div className="mt-2">
        <ModelChooser
          models={models}
          open={session.modelName}
          switchingTo={switchingTo}
          streaming={streaming}
          onSwitch={onSwitch}
        />
      </div>

      <div className="mt-2">
        {switching ? (
          /* The meter measures a window the loading model has not declared
             yet. A live one against the old session's denominator would be a
             fraction of the wrong number, so this says what is happening
             instead — and says it through `status`, because a switch takes
             seconds and a screen reader would otherwise announce nothing at
             all between the choice and the new session. */
          <p role="status" className="text-xs text-text-muted">
            Loading {switchingTo}… a model takes a few seconds to start. The conversation is kept.
          </p>
        ) : (
          <Meter
            label="Context"
            fraction={used / session.contextLength}
            detail={`${formatCount(used)} of ${formatCount(session.contextLength)} tokens`}
            warnWhenFull
          />
        )}
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
        <p className="mt-1 text-[11px] text-text-faint">
          This model&rsquo;s header declares no attention key length, so the KV-cache arithmetic
          derived one. That is correct for standard attention and wrong for models that diverge.
        </p>
      )}
    </section>
  );
}

/**
 * The control that swaps the open model for another downloaded one.
 *
 * A native `<select>` rather than a built menu, and deliberately: it is
 * keyboard-operable, labelled, type-ahead searchable and screen-reader
 * announced without a line of code written for any of it, and it costs no
 * dependency. Nothing here needs the two things a custom listbox would buy —
 * rich rows, or a search field — and the marker that has to survive is a string,
 * which an `<option>` carries perfectly well.
 *
 * **Disabled while a reply streams, rather than stopping the reply.** Both
 * satisfy "must be safe mid-generation"; they differ in what they cost when the
 * user did not mean it. Stopping first would truncate an answer they are
 * watching as a side effect of touching a control that looks like a label — an
 * unrecoverable loss from a reversible gesture. Refusing costs one click on the
 * Stop control that is already in this same bar while a reply streams, and says
 * so. What neither may do is switch underneath the stream: the transcript would
 * then attribute one model's words to another's name, which is the failure this
 * page exists to make impossible.
 */
function ModelChooser({
  models,
  open,
  switchingTo,
  streaming,
  onSwitch,
}: {
  models: readonly Downloaded[];
  /** The open session's model name. */
  open: string;
  /** The model a switch is loading, or `null` when none is. */
  switchingTo: string | null;
  streaming: boolean;
  onSwitch: (path: string) => void;
}): React.JSX.Element {
  // An empty control the user can open, tab into and find nothing in is a dead
  // end. The reason and the place to fix it are one sentence, so this says
  // both instead of rendering the control at all.
  if (models.length === 0) {
    return (
      <p className="text-[11px] text-text-muted">
        No models are downloaded, so there is nothing to switch to. The LLM tab is where models are
        found and fetched.
      </p>
    );
  }

  // Matched by name because that is all a session reports; the path it was
  // opened from is not part of `ModelSession`. A model opened from the file
  // box that is not in the library matches nothing, which is the case the
  // standing option below covers.
  const shown = switchingTo ?? open;
  const selected = models.find((model) => model.name === shown)?.path ?? '';

  return (
    <div className="flex flex-wrap items-center gap-2">
      <label htmlFor="chat-model" className="text-[10px] uppercase tracking-wider text-text-muted">
        Model
      </label>

      <select
        id="chat-model"
        value={selected}
        disabled={streaming || switchingTo !== null}
        onChange={(event) => {
          onSwitch(event.target.value);
        }}
        className="min-w-0 max-w-full rounded-md border border-edge bg-transparent px-2 py-0.5 font-mono text-xs disabled:opacity-40"
      >
        {/* The open model, when it is not one of the downloaded ones — which is
            what a file opened straight from the box above is. Without this the
            control would sit there displaying the first model in the list while
            a different one answered, which is the misattribution in a quieter
            form. Disabled because it is a statement, not a destination. */}
        {selected === '' && (
          <option value="" disabled>
            {shown} (not in the library)
          </option>
        )}

        {models.map((model) => (
          <option key={model.path} value={model.path}>
            {labelOf(model)}
          </option>
        ))}
      </select>

      {streaming && (
        <span className="text-[11px] text-text-muted">
          Stop the reply before switching, so the answer keeps the name of the model that wrote it.
        </span>
      )}
    </div>
  );
}

/**
 * One turn of the conversation.
 *
 * The user's turn and the model's have to be told apart at a glance while
 * scrolling past, so they differ three ways rather than one: the user's is
 * inset from the left and sits on `--color-surface` against the pane's
 * `--color-surface-raised`, while the model's is full width with an accent
 * gutter down its edge. One difference would be enough for someone reading
 * carefully; a transcript is read by skimming.
 *
 * Colours come from the theme tokens, not from literals, so a theme change
 * moves the transcript with everything else.
 */
function Turn({
  message,
  timings = null,
}: {
  message: Message;
  timings?: Timings | null;
}): React.JSX.Element {
  const speeds = timings;
  const fromUser = message.role === 'user';

  return (
    <li
      className={`flex flex-col gap-1 rounded-lg px-2.5 py-2 ${
        fromUser
          ? 'ml-6 border border-edge bg-surface'
          : 'mr-6 border-l-2 border-l-accent/50 bg-surface-raised'
      }`}
    >
      <div className="flex items-baseline gap-2">
        <span
          className={`text-[10px] uppercase tracking-wider ${
            fromUser ? 'text-text-muted' : 'text-accent/80'
          }`}
        >
          {ROLE_LABEL[message.role]}
        </span>

        {/* Absent rather than guessed when the turn predates the field, or
            when the clock could not be read. A time that was never recorded
            must not be drawn as the epoch. */}
        {message.sentAt !== null && (
          <time
            dateTime={new Date(message.sentAt).toISOString()}
            className="font-mono text-[10px] text-text-muted"
          >
            {formatTimeOfDay(message.sentAt)}
          </time>
        )}
      </div>

      {/* Both roles read at full strength. The old code separated them by one
          step of grey, which the role label above already says outright and in
          colour — and a transcript where half the turns are dimmer is just
          harder to read for no gain. */}
      <div data-selectable className="text-sm text-text">
        {renderText(message.content)}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        {message.usage !== null && (
          <span data-selectable className="font-mono text-[11px] text-text-faint">
            {`${formatCount(message.usage.promptTokens)} in · ${formatCount(
              message.usage.completionTokens
            )} out`}
          </span>
        )}

        {/* One decimal, because a reply is over in single-digit seconds often
            enough that whole seconds would round half the answers to the same
            number. `formatDuration` is for uptime and floors to the second,
            which is why this does not use it. */}
        {message.elapsedSeconds !== null && (
          <span data-selectable className="font-mono text-[11px] text-text-faint">
            {`${message.elapsedSeconds.toFixed(1)} s`}
          </span>
        )}

        {message.stopped && <span className="text-[11px] text-amber-400/80">Stopped</span>}

        {speeds !== null && speeds.promptPerSecond !== null && (
          <span className="font-mono text-[11px] text-text-faint">
            {`${speeds.promptPerSecond.toFixed(1)} tok/s prompt`}
          </span>
        )}

        {speeds !== null && speeds.predictedPerSecond !== null && (
          <span className="font-mono text-[11px] text-text-muted">
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
  switching,
  onDraftChange,
  onSend,
}: {
  draft: string;
  streaming: boolean;
  /** Whether a model switch is loading, which is also nothing to talk to. */
  switching: boolean;
  onDraftChange: (value: string) => void;
  onSend: () => void;
}): React.JSX.Element {
  // Two reasons the composer is closed and two sentences for them. "Waiting for
  // the reply" during a model load would name a reply that does not exist.
  const busy = streaming || switching;
  const placeholder = streaming
    ? 'Waiting for the reply…'
    : switching
      ? 'Loading the model…'
      : 'Say something';

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
        disabled={busy}
        placeholder={placeholder}
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
        disabled={busy || draft.trim() === ''}
        className="rounded-md border border-accent px-3 py-1.5 text-xs text-accent transition-colors hover:bg-accent/10 disabled:opacity-40"
      >
        Send
      </button>
    </form>
  );
}
