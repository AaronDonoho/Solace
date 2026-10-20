// The root project's workflow, as `GET /workflow` answers it (D-140): the
// Knowledgebase's column table follows it. A failed request answers with the
// error's message, which is no workflow: the guide then falls back to its
// known columns.
export function load_workflow(receive) {
  fetch('/workflow')
    .then((response) => response.text())
    .then(receive)
    .catch((error) => receive(JSON.stringify({ message: String(error) })));
}
