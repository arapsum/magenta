use async_channel::Receiver;
use futures_util::StreamExt as _;
use magenta_core::{
    AgentProviderEvent, AgentRequest, AgentResumeRequest, AgentRunEvent, AgentRunStream,
};

use super::{
    AgentStreamContext, ApprovalResponse, MAX_AGENT_ROUNDS, MAX_AGENT_TOOL_CALLS, agent_error,
    tools,
};

pub fn agent_stream(
    context: AgentStreamContext,
    request: AgentRequest,
    approvals: Receiver<ApprovalResponse>,
) -> AgentRunStream {
    Box::pin(async_stream::try_stream! {
        let provider_id = request.generation.provider.clone();
        let mut provider_stream = context.provider.start(request.clone());
        let mut rounds = 0_usize;
        let mut tool_calls = 0_usize;
        let mut first_started = false;

        loop {
            let mut calls = Vec::new();
            let mut continuation = None;
            while let Some(event) = provider_stream.next().await {
                match event? {
                    AgentProviderEvent::Started => {
                        if !first_started {
                            first_started = true;
                            yield AgentRunEvent::Started;
                        }
                    }
                    AgentProviderEvent::TextDelta(delta) => {
                        yield AgentRunEvent::TextDelta(delta);
                    }
                    AgentProviderEvent::ToolCall { call, continuation: next } => {
                        calls.push(call.clone());
                        continuation = Some(next);
                        yield AgentRunEvent::ToolCall(call);
                    }
                    AgentProviderEvent::Completed(outcome) => {
                        yield AgentRunEvent::Completed(outcome);
                        return;
                    }
                }
            }

            let Some(continuation) = continuation else {
                Err::<(), _>(agent_error(
                    &provider_id,
                    "provider ended an agent step without a completion or tool call",
                ))?;
                unreachable!();
            };
            rounds += 1;
            tool_calls += calls.len();
            if rounds > MAX_AGENT_ROUNDS || tool_calls > MAX_AGENT_TOOL_CALLS {
                Err::<(), _>(agent_error(&provider_id, "agent tool-call limit reached"))?;
            }

            let mut outputs = Vec::with_capacity(calls.len());
            let mut tool_stream = tools::execute_tools(
                context.clone(),
                calls,
                approvals.clone(),
                provider_id.clone(),
            );
            while let Some(event) = tool_stream.next().await {
                let event = event?;
                if let AgentRunEvent::ToolResult(output) = &event {
                    outputs.push(output.clone());
                }
                yield event;
            }

            provider_stream = context.provider.resume(AgentResumeRequest {
                continuation,
                outputs,
                instructions: request.instructions.clone(),
                tools: request.tools.clone(),
            });
        }
    })
}
