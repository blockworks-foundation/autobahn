RandBot
=======

Randomly trade to stress test router, while trying to keep wallet SOL/USDC only.

On each call, will swap N USDC to A, then A to B, then B to USDC.
- N is randomly selected
- A and B are randomly selected 

On start, if there is an existing exposure, will swap eveyrthing to USDC (keeping some SOL for gas)

