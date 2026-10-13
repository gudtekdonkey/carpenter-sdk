const { treasuryExports } = require('../helper/treasury')

// Carpenter's protocol treasury: the carpenter_amm program's ["protocol_treasury"] account, which owns the token accounts
// the protocol fee is swept into.
module.exports = treasuryExports({
  solana: {
    owners: ['G7URMTZpyVendaTXEki7emtvvB1UirZFfdvruYy8h66r'],
  },
})
