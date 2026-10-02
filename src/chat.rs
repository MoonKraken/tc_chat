use topcoat::{
    Result,
    context::{Cx, app_context},
    router::page,
    runtime::{Event, connected, procedure, shard, signal},
    view::{Unescaped, View, emit, live, view},
};
use uuid::Uuid;

use crate::{hub::ChatHub, markdown};

// Text nodes are HTML-escaped, and a browser does not decode entities inside
// <script>. Keep this source free of `&`, `<`, and `>`.
const SCROLL_TO_LATEST: &str = r#"(function () {
  var scroller = document.getElementById("transcript");
  if (!scroller || scroller.dataset.bound === "1") return;
  scroller.dataset.bound = "1";
  var follow = true;
  function nearLatest() {
    var gap = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    return Math.min(gap, 80) === gap;
  }
  scroller.addEventListener("scroll", function () {
    follow = nearLatest();
  });
  new MutationObserver(function () {
    if (follow) scroller.scrollTop = scroller.scrollHeight;
  }).observe(scroller, { childList: true, subtree: true, characterData: true });
})();"#;

#[page("/")]
pub async fn home(cx: &Cx) -> Result<impl View> {
    // A new id on each page load. Refresh starts an empty transcript.
    let id = Uuid::new_v4().to_string();
    app_context::<ChatHub>(cx).open(&id);

    let conversation_id = signal(cx, move || id);
    let draft = signal(cx, String::new);
    let notice = signal(cx, String::new);
    let busy = signal(cx, || false);

    Ok(view! {
        <div
            id="transcript"
            role="log"
            aria-live="polite"
            aria-relevant="additions"
            class="min-h-0 flex-1 overflow-y-auto"
        >
            transcript(conversation: $(conversation_id.get()))
        </div>

        <form
            class="border-t border-zinc-700 py-4"
            @submit=$(async |e: Event| {
                e.prevent_default();
                // `return` inside `if` leaves only that expression's wrapper.
                if busy.get() {
                } else {
                    if draft.get().trim().is_empty() {
                    } else {
                        busy.set(true);
                        notice.set("".to_owned());
                        let text = draft.get();
                        draft.set("".to_owned());
                        let status = send(
                            conversation_id.get().to_owned(),
                            text.to_owned(),
                        )
                        .await;
                        if status.is_empty() {
                            notice.set("".to_owned());
                        } else {
                            draft.set(text);
                            notice.set(status);
                        }
                        busy.set(false);
                    }
                }
            })
        >
            <label class="sr-only" for="composer">"Message"</label>
            <div class="rounded-3xl bg-zinc-700 p-2 shadow-sm ring-1 ring-zinc-600 focus-within:ring-blue-500">
                <textarea
                    id="composer"
                    rows="3"
                    maxlength="8000"
                    placeholder="Write a message"
                    class="block w-full resize-none bg-transparent px-3 py-2 text-base text-white outline-none placeholder:text-zinc-400"
                    :value=$(draft.get())
                    @input=$(|e: Event| draft.set(e.target.value))
                    @keydown=$(async |e: Event| {
                        if e.key == "Enter" {
                            if e.shift_key {
                            } else {
                                if e.is_composing {
                                } else {
                                    e.prevent_default();
                                    if busy.get() {
                                    } else {
                                        if draft.get().trim().is_empty() {
                                        } else {
                                            busy.set(true);
                                            notice.set("".to_owned());
                                            let text = draft.get();
                                            draft.set("".to_owned());
                                            let status = send(
                                                conversation_id.get().to_owned(),
                                                text.to_owned(),
                                            )
                                            .await;
                                            if status.is_empty() {
                                                notice.set("".to_owned());
                                            } else {
                                                draft.set(text);
                                                notice.set(status);
                                            }
                                            busy.set(false);
                                        }
                                    }
                                }
                            }
                        }
                    })
                ></textarea>
                <div class="flex items-center justify-between gap-3 px-2 pb-1">
                    <p class="text-xs text-zinc-400">"Enter to send · Shift+Enter for a new line"</p>
                    <button
                        type="submit"
                        class="rounded-full bg-green-700 px-4 py-1.5 text-sm font-semibold text-white hover:bg-green-600 disabled:cursor-not-allowed disabled:bg-zinc-600 disabled:text-zinc-400"
                        :disabled=$(if busy.get() {
                            true
                        } else {
                            draft.get().trim().is_empty()
                        })
                    >
                        $(if busy.get() { "Sending…" } else { "Send" })
                    </button>
                </div>
            </div>
            <p
                class="mt-2 min-h-5 px-3 text-sm text-red-400"
                :hidden=$(notice.get().is_empty())
                role="status"
            >
                $(notice.get())
            </p>
        </form>

        <script>(SCROLL_TO_LATEST)</script>
    })
}

#[shard]
async fn transcript(cx: &Cx, conversation: String) -> Result<impl View> {
    Ok(live! {
        let hub = app_context::<ChatHub>(cx);
        let mut changed = hub.open(&conversation).subscribe_updated();

        loop {
            let lines = hub.open(&conversation).lines();
            let token = emit! {
                <div class="flex min-h-full flex-col gap-5 py-6">
                    if lines.is_empty() {
                        <div class="m-auto max-w-md px-2 text-center">
                            <p class="text-xl font-semibold text-zinc-100">
                                "Welcome to tc-chat!"
                            </p>
                            <p class="mt-2 text-sm leading-6 text-zinc-400">
                                "Write a prompt in the message field below!"
                            </p>
                        </div>
                    }
                    #[key(line.id)]
                    for line in lines {
                        <article
                            class=(if line.mine {
                                "ml-auto max-w-[85%] rounded-2xl rounded-br-md bg-blue-500 px-4 py-3 text-white"
                            } else {
                                "mr-auto max-w-[85%] rounded-2xl rounded-bl-md bg-zinc-700 px-4 py-3 text-white"
                            })
                        >
                            <p class="mb-1 text-[0.7rem] font-semibold uppercase tracking-wider opacity-70">
                                (if line.mine { "You" } else { "Assistant" })
                            </p>
                            if line.pending && line.text.is_empty() {
                                <p class="flex items-center gap-1 py-1" aria-label="The assistant is writing">
                                    <span class="size-1.5 animate-bounce rounded-full bg-current [animation-delay:0ms]"></span>
                                    <span class="size-1.5 animate-bounce rounded-full bg-current [animation-delay:150ms]"></span>
                                    <span class="size-1.5 animate-bounce rounded-full bg-current [animation-delay:300ms]"></span>
                                </p>
                            } else {
                                <div class="md">
                                    (Unescaped::new_unchecked(markdown::render(&line.text)))
                                </div>
                                if line.pending {
                                    <span class="mt-1 inline-block h-4 w-px animate-pulse bg-current align-middle"></span>
                                }
                            }
                        </article>
                    }
                </div>
            }?;

            if !connected(cx) {
                break Ok(token);
            }
            changed.recv().await.ok();
        }
    })
}

#[procedure]
pub async fn send(cx: &Cx, conversation: String, message: String) -> Result<String> {
    if Uuid::parse_str(&conversation).is_err() {
        return Ok("Send that message again.".into());
    }
    match app_context::<ChatHub>(cx)
        .respond(&conversation, &message)
        .await
    {
        Ok(()) => Ok(String::new()),
        Err(reason) => Ok(reason),
    }
}
