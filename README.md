# ORE

ORE is a crypto mining protocol.


## API
- [`Consts`](api/src/consts.rs) – Program constants.
- [`Error`](api/src/error.rs) – Custom program errors.
- [`Event`](api/src/error.rs) – Custom program events.
- [`Instruction`](api/src/instruction.rs) – Declared instructions and arguments.

## Instructions

#### Mining
- [`Automate`](program/src/automate.rs) - Configures a new automation.
- [`Checkpoint`](program/src/checkpoint.rs) - Checkpoints rewards from an prior round.
- [`ClaimORE`](program/src/claim_ore.rs) - Claims ORE mining rewards.
- [`ClaimSOL`](program/src/claim_sol.rs) - Claims SOL mining rewards.
- [`Deploy`](program/src/deploy.rs) – Deploys SOL to claim space on the board.
- [`Initialize`](program/src/initialize.rs) - Initializes program variables.
- [`Log`](program/src/log.rs) – Logs non-truncatable event data.
- [`Reset`](program/src/reset.rs) - Resets the board for a new round.
- [`Reset`](program/src/reset.rs) - Resets the board for a new round.

#### Staking
- [`Deposit`](program/src/deposit.rs) - Deposits ORE into a stake account.
- [`Withdraw`](program/src/withdraw.rs) - Withdraws ORE from a stake account.
- [`ClaimSeeker`](program/src/claim_seeker.rs) - Claims a Seeker genesis token. 
- [`ClaimYield`](program/src/claim_yield.rs) - Claims staking yield.

#### Admin
- [`Bury`](program/src/bury.rs) - Executes a buy-and-bury transaction.
- [`Wrap`](program/src/wrap.rs) - Wraps SOL in the treasury for swap transactions. 
- [`SetAdmin`](program/src/set_admin.rs) - Re-assigns the admin authority.
- [`SetFeeCollector`](program/src/set_admin.rs) - Updates the fee collection address.
- [`SetFeeRate`](program/src/set_admin.rs) - Updates the fee charged per swap.

## State
- [`Automation`](api/src/state/automation.rs) - Tracks automation configs. 
- [`Board`](api/src/state/board.rs) - Tracks the current round number and timestamps.
- [`Config`](api/src/state/config.rs) - Global program configs.
- [`Miner`](api/src/state/miner.rs) - Tracks a miner's game state.
- [`Round`](api/src/state/round.rs) - Tracks the game state of a given round.
- [`Seeker`](api/src/state/seeker.rs) - Tracks whether a Seeker token has been claimed.
- [`Stake`](api/src/state/stake.rs) - Manages a user's staking activity.
- [`Treasury`](api/src/state/treasury.rs) - Mints, burns, and escrows ORE tokens. 


## HTTP API

The CLI provides an HTTP server with RESTful API endpoints for interacting with the ORE protocol.

### Endpoints

- `GET /health` - Health check
- `GET /api/round` - Get current round data
- `GET /api/board` - Get current board data
- `GET /api/data` - Get combined round and board data
- `GET /api/winning-tiles` - Get winning tiles statistics
- `GET /api/gmore-state` - Get gmore state from Redis
- `POST /api/deploy` - Create deploy instruction

### Deploy Instruction API

The `/api/deploy` endpoint creates a deploy instruction that can be directly used by frontend applications to build and sign transactions.

#### Request

```bash
POST http://localhost:8080/api/deploy
Content-Type: application/json

{
  "squares": [true, false, false, ...],  // Array of 25 booleans
  "authority": "YOUR_SOLANA_ADDRESS",     // Base58 encoded Solana address
  "round_id": 55297,                      // Round ID
  "amount": 100000000                     // Amount in lamports
}
```

#### Response

```json
{
  "programId": "program_id_base58",
  "accounts": [
    {
      "pubkey": "account_address_base58",
      "isSigner": true,
      "isWritable": true
    }
  ],
  "data": "base64_encoded_instruction_data"
}
```

#### Frontend Usage Example

```javascript
import { Transaction, PublicKey } from '@solana/web3.js';
import { Connection } from '@solana/web3.js';

// 1. Call API to get deploy instruction
async function createDeployInstruction(squares, authority, roundId, amount) {
  const response = await fetch('http://localhost:8080/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      squares: squares,        // Array of 25 booleans
      authority: authority,    // Solana address (base58)
      round_id: roundId,       // Round ID
      amount: amount           // Amount in lamports
    })
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.message || 'Failed to create deploy instruction');
  }

  return await response.json();
}

// 2. Build transaction and sign
async function deploy(connection, wallet, squares, roundId, amount) {
  try {
    // Get instruction from API
    const instructionData = await createDeployInstruction(
      squares,
      wallet.publicKey.toBase58(),
      roundId,
      amount
    );

    // Build transaction
    const transaction = new Transaction();
    transaction.add({
      programId: new PublicKey(instructionData.programId),
      keys: instructionData.accounts.map(acc => ({
        pubkey: new PublicKey(acc.pubkey),
        isSigner: acc.isSigner,
        isWritable: acc.isWritable,
      })),
      data: Buffer.from(instructionData.data, 'base64'),
    });

    // Get recent blockhash
    const { blockhash } = await connection.getLatestBlockhash();
    transaction.recentBlockhash = blockhash;
    transaction.feePayer = wallet.publicKey;

    // Sign and send transaction
    const signature = await wallet.sendTransaction(transaction, connection);
    await connection.confirmTransaction(signature, 'confirmed');

    return signature;
  } catch (error) {
    console.error('Deploy failed:', error);
    throw error;
  }
}

// Usage example
const squares = [
  true, false, false, false, false,
  false, false, false, false, false,
  false, false, false, false, false,
  false, false, false, false, false,
  false, false, false, false, false
];

deploy(
  connection,           // Solana connection
  wallet,               // Wallet adapter
  squares,              // 25 boolean array
  55297,                // Round ID
  100000000             // Amount in lamports (0.1 SOL)
).then(signature => {
  console.log('Transaction signature:', signature);
});
```

#### TypeScript Example

```typescript
import { Transaction, PublicKey, Connection } from '@solana/web3.js';
import { WalletContextState } from '@solana/wallet-adapter-react';

interface DeployRequest {
  squares: boolean[];
  authority: string;
  round_id: number;
  amount: number;
}

interface AccountMeta {
  pubkey: string;
  isSigner: boolean;
  isWritable: boolean;
}

interface DeployInstructionResponse {
  programId: string;
  accounts: AccountMeta[];
  data: string;
}

async function createDeployInstruction(
  squares: boolean[],
  authority: string,
  roundId: number,
  amount: number
): Promise<DeployInstructionResponse> {
  const response = await fetch('http://localhost:8080/api/deploy', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      squares,
      authority,
      round_id: roundId,
      amount,
    } as DeployRequest),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(error.message || 'Failed to create deploy instruction');
  }

  return await response.json();
}

export async function deployToBoard(
  connection: Connection,
  wallet: WalletContextState,
  squares: boolean[],
  roundId: number,
  amount: number
): Promise<string> {
  if (!wallet.publicKey || !wallet.signTransaction) {
    throw new Error('Wallet not connected');
  }

  const instructionData = await createDeployInstruction(
    squares,
    wallet.publicKey.toBase58(),
    roundId,
    amount
  );

  const transaction = new Transaction();
  transaction.add({
    programId: new PublicKey(instructionData.programId),
    keys: instructionData.accounts.map(acc => ({
      pubkey: new PublicKey(acc.pubkey),
      isSigner: acc.isSigner,
      isWritable: acc.isWritable,
    })),
    data: Buffer.from(instructionData.data, 'base64'),
  });

  const { blockhash } = await connection.getLatestBlockhash();
  transaction.recentBlockhash = blockhash;
  transaction.feePayer = wallet.publicKey;

  const signedTransaction = await wallet.signTransaction(transaction);
  const signature = await connection.sendRawTransaction(
    signedTransaction.serialize()
  );
  await connection.confirmTransaction(signature, 'confirmed');

  return signature;
}
```

## Tests

To run the test suite, use the Solana toolchain: 

```
cargo test-sbf
```

For line coverage, use llvm-cov:

```
cargo llvm-cov
```
