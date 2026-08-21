# Desktop

The Desktop context owns Nest's local knowledge workspace and the user-facing chat experience over that workspace.

## Language

**Chat Backend**:
The runtime that produces a chat response and conducts any model-driven tool loop. Nest Agent and Claude Agent are Chat Backends.
_Avoid_: Provider, engine, chat mode

**Chat Mode**:
The per-session capability policy applied to a Chat Backend. Ask permits read-only responses; Agent permits reviewable knowledge-change proposals.
_Avoid_: Backend, provider, agent selector

**Nest Agent**:
The built-in Chat Backend implemented with Rig and an OpenAI-compatible model endpoint.
_Avoid_: Default provider, OpenAI mode

**Claude Agent**:
The external Chat Backend implemented by invoking an authenticated Claude CLI installation.
_Avoid_: Claude mode, Claude provider

**Knowledge Capability**:
A Nest-controlled operation that lets a Chat Backend retrieve from or propose changes to the local knowledge workspace. Claude Agent receives Knowledge Capabilities only through an explicit Nest integration.
_Avoid_: Native Claude tool, direct vault access

## Delivery Stages

**Step 1**:
The Windows-only delivery stage in which an enabled Claude Agent handles chat and a disabled Claude Agent leaves chat on Nest Agent. It adds configuration and connection verification but no Knowledge Capabilities or automatic Claude titles; Ask and Agent remain visible but have identical behavior on Claude Agent.

**Step 2**:
The delivery stage that adds Chat Backend and model selection and exposes Nest Knowledge Capabilities to Claude Agent.

**Claude Session Binding**:
The one-to-one association in which a Nest chat session UUID is also the Claude session ID used to resume its conversation. A bound Nest chat never changes to a different Claude session.
_Avoid_: Continue latest session, shared Claude session

**Backend Binding**:
The Chat Backend chosen atomically when the first message is sent in an unbound Nest chat session. A bound session keeps its Chat Backend; changes to the enabled default apply only to new, unbound chats.
_Avoid_: Per-message routing, implicit backend switching
