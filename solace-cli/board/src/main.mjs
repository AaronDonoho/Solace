// Nothing drags (links, images, selected text) except text inside an editor.
document.addEventListener('dragstart', e => e.target.closest?.('input,textarea') || e.preventDefault());
if (location.pathname.startsWith('/view')) {
  document.title = 'Solace contract view';
  import('../build/dev/javascript/solace_board/solace_view.mjs').then(({ main }) => main());
} else {
  import('../build/dev/javascript/solace_board/solace_board.mjs').then(({ main }) => main());
}
