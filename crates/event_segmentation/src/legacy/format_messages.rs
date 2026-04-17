use plastmem_shared::Message;

pub fn format_messages(messages: &[Message]) -> String {
  messages
    .iter()
    .enumerate()
    .map(|(i, m)| {
      format!(
        "[{}] {} [{}] {}",
        i,
        m.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        m.role,
        m.content
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}
