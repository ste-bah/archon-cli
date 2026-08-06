//! Transient provider-failure retry.
//!
//! The logic moved to [`archon_workflow::llm_retry`], beside the LLM port it
//! decorates: the retry budget and the transient-error vocabulary are facts
//! about how the workflow runtime uses a provider, not about which provider
//! this binary installed. What is left here is the name every live call site
//! already reaches through, so the move did not touch them.

pub(crate) use archon_workflow::llm_retry::send_message_with_transient_retry;
