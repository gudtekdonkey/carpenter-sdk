const { PublicKey } = require('@solana/web3.js')
const { getConnection, sumTokens2 } = require('../helper/solana')

// Carpenter (https://github.com/gudtekdonkey/carpenter-sdk): a concentrated-liquidity AMM on Solana with
// Uniswap-v4-style hooks. Every pool holds its two tokens in its own vault accounts; TVL is their balances.
const CARPENTER_AMM = new PublicKey('DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA')
// The Pool account's discriminator, f19a6d0411b16dbc, in base58.
const POOL_DISCRIMINATOR = 'hQrXeCntzbV'
// Pool.vault0 / Pool.vault1, after the 8-byte discriminator.
const VAULT0 = 8 + 152
const VAULT1 = 8 + 184

async function tvl(api) {
  const pools = await getConnection().getProgramAccounts(CARPENTER_AMM, {
    filters: [{ memcmp: { offset: 0, bytes: POOL_DISCRIMINATOR } }],
    dataSlice: { offset: VAULT0, length: 64 },
  })
  const tokenAccounts = []
  for (const { account } of pools) {
    tokenAccounts.push(new PublicKey(account.data.subarray(0, 32)).toBase58())
    tokenAccounts.push(new PublicKey(account.data.subarray(VAULT1 - VAULT0, VAULT1 - VAULT0 + 32)).toBase58())
  }
  return sumTokens2({ api, tokenAccounts })
}

module.exports = {
  methodology: 'TVL is the balance of every Carpenter pool\'s two token vaults.',
  solana: { tvl },
}
