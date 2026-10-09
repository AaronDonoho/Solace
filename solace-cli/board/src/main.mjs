// The wrapper shell owns routing. This page mounts the board for standalone use.
import('../build/dev/javascript/solace_board/solace_board.mjs').then(({ register }) => register());
