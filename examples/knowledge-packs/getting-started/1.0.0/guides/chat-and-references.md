# Chat and `@` References

## Choose an agent and model

The chat box starts with three selectors: **Agent**, **Model**, and **Mode**.

- **Agent** picks the backend for the conversation. **Nest Agent** uses the
  model configured in Settings; **Claude** routes the chat through your local
  Claude CLI once it is enabled and connected in Settings. The choice applies
  to a new conversation the first time you send a message; from then on that
  conversation keeps its backend. Switching the agent mid-conversation starts
  a new chat and carries your unsent draft over.
- **Model** shows the model you'll get: the configured API model for Nest,
  or for Claude the model the CLI currently uses (marked `(default)`) plus
  any custom models you listed in Settings.
- **Mode** is **Ask** or **Agent** as described below.

## Ask across active packs

Use the selector at the lower-left of the chat box to choose a mode:

- **Ask** answers questions and never edits files.
- **Agent** can edit Markdown files in active packs when the task calls for it. Selecting Agent grants permission for the turn, but the model may still answer without changing anything.

While Agent works, Nest shows the file being edited or staged. After the reply, the Markdown viewer previews proposed content immediately. Expand **files changed**, then choose **Review in editor** for an inline diff with **Approve** and **Reject**. The vault changes only when you approve. Cancelling or rejecting applies none of the proposal. Files already open in the editor and packs you cannot maintain remain protected.

You can continue giving Agent tasks before reviewing. Agent treats unresolved proposals as the current version of those files, so follow-up edits build on the preview instead of the original disk content.

A prompt with no reference searches all active packs. Be specific about the
result, audience, and format you want.

## Focus with `@`

Type `@`, pick a file or folder, and it appears as a pill limiting retrieval
to that reference. Combine multiple references.

```text
@getting-started/guides/editing-and-source-control.md
Give me a checklist for reviewing edits before publishing.
```

## Check citations

Open the citations under a response when the answer seems incomplete or
wrong, then rephrase or narrow your `@` reference.

## Tips

1. Keep only relevant packs active.
2. Ask one concrete task at a time.
3. State the audience and output format.
4. Verify important statements against citations.
