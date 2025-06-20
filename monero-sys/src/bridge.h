#pragma once

#include <memory>

#include "../monero/src/wallet/api/wallet2_api.h"
#include "../monero/src/wallet/api/wallet_manager.h"

/**
 * This file contains some C++ glue code needed to make the FFI work.
 * This consists mainly of two use-cases:
 *
 *  1. Work arounds around CXX limitations.
 *  2. Hooking into the C++ logging system to forward the messages to Rust.
 *
 *  1. Work arounds:
 *     - CXX doesn't support static methods as yet, so we define free functions here that
 *       simply call the appropriate static methods.
 *     - CXX also doesn't support returning strings by value from C++ to Rust, so we wrap
 *       those in a unique_ptr.
 *     - CXX doesn't support optional arguments, so we make thin wrapper functions that either
 *       take the argument or not.
 *
 *  2. Hooking into the C++ logging system:
 *     - We install a custom callback to the easylogging++ logging system that forwards
 *       all log messages to Rust.
 */

/**
 * This adds some glue to the Monero namespace to make the ffi work.
 * Mostly work arounds for CXX limitations.
 */
namespace Monero
{
    using ConnectionStatus = Wallet::ConnectionStatus;

    /**
     * CXX doesn't support static methods as yet, so we define free functions here that simply
     * call the appropriate static methods.
     */
    inline WalletManager *getWalletManager()
    {
        // This causes the wallet start logging.
        // This is useful for debugging.
        // We enable the maximum log level since we capture
        // and forward the logs to tracing anyway, which has a seperate level control
        WalletManagerFactory::setLogLevel(4);

        auto *manager = Monero::WalletManagerFactory::getWalletManager();
        return manager;
    }

    /**
     * CXX also doesn't support returning strings by value from C++ to Rust, so we wrap those
     * in a unique_ptr.
     */
    inline std::unique_ptr<std::string> address(const Wallet &wallet, uint32_t account_index, uint32_t address_index)
    {
        auto addr = wallet.address(account_index, address_index);
        return std::make_unique<std::string>(addr);
    }

    /**
     * Same as for [`address`]
     */
    inline std::unique_ptr<std::string> walletManagerErrorString(WalletManager &manager)
    {
        auto err = manager.errorString();
        return std::make_unique<std::string>(err);
    }

    /**
     * Get the error string of a pending transaction.
     */
    inline std::unique_ptr<std::string> pendingTransactionErrorString(const PendingTransaction &tx)
    {
        auto err = tx.errorString();
        return std::make_unique<std::string>(err);
    }

    /**
     * Wrapper for Wallet::checkTxKey to accommodate passing std::string by reference.
     * The original API takes the tx_key parameter by value which is not compatible
     * with cxx. Taking it by const reference here allows us to expose the function
     * to Rust safely while still calling the original method internally.
     */
    inline bool checkTxKey(
        Wallet &wallet,
        const std::string &txid,
        const std::string &tx_key,
        const std::string &address,
        uint64_t &received,
        bool &in_pool,
        uint64_t &confirmations)
    {
        return wallet.checkTxKey(txid, tx_key, address, received, in_pool, confirmations);
    }

    inline bool scanTransaction(Wallet &wallet, const std::string &txid)
    {
        std::vector<std::string> txids;
        txids.push_back(txid.c_str());
        return wallet.scanTransactions(txids);
    }

    /**
     * Get the path of the wallet.
     */
    inline std::unique_ptr<std::string> walletPath(const Wallet &wallet)
    {
        return std::make_unique<std::string>(wallet.path());
    }

    /**
     * A wrapper around Wallet::createTransaction which passes sensible defaults and doesn't
     * require an optional argument which CXX doesn't support.
     */
    inline PendingTransaction *createTransaction(
        Wallet &wallet,
        const std::string &dest_address,
        uint64_t amount)
    {
        return wallet.createTransaction(dest_address, "", Monero::optional<uint64_t>(amount), 0, PendingTransaction::Priority_Default);
    }

    /**
     * Create a transaction that spends all the unlocked balance to a single destination.
     */
    inline PendingTransaction *createSweepTransaction(
        Wallet &wallet,
        const std::string &dest_address)
    {
        return wallet.createTransaction(dest_address, "", Monero::optional<uint64_t>(), 0, PendingTransaction::Priority_Default);
    }

    /**
     * Creates a transaction that spends the unlocked balance to multiple destinations with given ratios.
     * Ratiosn must sum to 1.
     */
    inline PendingTransaction *createMultiSweepTransaction(
        Wallet &wallet,
        const std::vector<std::string> &dest_addresses,
        const std::vector<double> &sweep_ratios) // Sweep ratios must sum to 1
    {
        size_t n = dest_addresses.size();

        // Check if we have any destinations at all
        if (n == 0)
        {
            // wallet.setStatusError("Number of destinations must be greater than 0");
            return nullptr;
        }

        // Check if the number of destinations and sweep ratios match
        if (sweep_ratios.size() != n)
        {
            // wallet.setStatusError("Number of destinations and sweep ratios must match");
            return nullptr;
        }

        // Check if the sweep ratios sum to 1
        double sum_ratios = 0.0;
        for (const auto &ratio : sweep_ratios)
        {
            sum_ratios += ratio;
        }

        const double epsilon = 1e-6;
        if (std::abs(sum_ratios - 1.0) > epsilon)
        {
            // wallet.setStatusError("Sweep ratios must sum to 1.0");
            return nullptr;
        }

        // To estimate the correct fee we build a transation that spends 1 piconero
        // to (N-1) destinations and 1 change output (which is created by the wallet by default)

        // Build temporary destination list for fee estimation
        // Skip the last destination because for the fee estimation the
        // change output will serve as the last destination
        std::vector<std::pair<std::string, uint64_t>> fee_dests;
        fee_dests.reserve(n - 1);
        for (size_t i = 0; i < n - 1; ++i)
            fee_dests.emplace_back(dest_addresses[i], 1);

        // Estimate fee assuming N-1 outputs + 1 change
        uint64_t fee = wallet.estimateTransactionFee(fee_dests, PendingTransaction::Priority_Default);

        uint64_t balance = wallet.unlockedBalance();
        uint64_t sweepable_balance = balance - fee;

        // Now split that sweepable balance into N parts based on the sweep ratios
        std::vector<uint64_t> amounts(n);

        // First N-1 outputs are split based on the sweep ratios
        uint64_t sum = 0;
        for (size_t i = 0; i < n - 1; ++i)
        {
            uint64_t amount = static_cast<uint64_t>(std::floor(sweepable_balance * sweep_ratios[i]));

            // If the amount is 0, throw an error
            if (amount == 0)
            {
                // wallet.setStatusError("One of destination addresses would receive 0 piconero");
                return nullptr;
            }

            amounts[i] = amount;
            sum += amount;
        }

        // Last output is the remaining balance
        // This accounts for small rounding errors
        // which cannot be avoided when using floating point arithmetic
        amounts[n - 1] = sweepable_balance - sum;

        // Assert that the sum of the amounts is equal to the sweepable balance
        sum = 0;
        for (size_t i = 0; i < n; ++i)
            sum += amounts[i];
        if (sum != sweepable_balance)
        {
            // wallet.setStatusError("Failed to split balance into parts");
            return nullptr;
        }

        // Build the actual multi‐dest transaction
        // No change left -> wallet drops it
        // N outputs, fee should be the same as the one estimated above
        return wallet.createTransactionMultDest(
            dest_addresses,
            "", // No Payment ID
            Monero::optional<std::vector<uint64_t>>(amounts),
            0, // No mixin count
            PendingTransaction::Priority_Default);
    }

    inline bool setWalletDaemon(Wallet &wallet, const std::string &daemon_address)
    {
        return wallet.setDaemon(daemon_address);
    }

    inline std::unique_ptr<std::string> pendingTransactionTxId(const PendingTransaction &tx)
    {
        const auto ids = tx.txid();
        if (ids.empty())
            return std::make_unique<std::string>("");
        return std::make_unique<std::string>(ids.front());
    }

    /**
     * Get the transaction key for a given transaction id
     */
    inline std::unique_ptr<std::string> walletGetTxKey(const Wallet &wallet, const std::string &txid)
    {
        auto key = wallet.getTxKey(txid);
        return std::make_unique<std::string>(key);
    }

    /**
     * Get the seed of the wallet.
     */
    inline std::unique_ptr<std::string> walletSeed(const Wallet &wallet, const std::string &seed_offset)
    {
        auto seed = wallet.seed(seed_offset);
        return std::make_unique<std::string>(seed);
    }

    inline std::unique_ptr<std::vector<std::string>> pendingTransactionTxIds(const PendingTransaction &tx)
    {
        return std::make_unique<std::vector<std::string>>(tx.txid());
    }

    inline std::unique_ptr<std::string> walletFilename(const Wallet &wallet)
    {
        return std::make_unique<std::string>(wallet.filename());
    }

    inline void vector_string_push_back(
        std::vector<std::string> &v,
        const std::string &s)
    {
        v.push_back(s);
    }
}

#include "easylogging++.h"
#include "bridge.h"
#include "monero-sys/src/bridge.rs.h"

/**
 * This section is us capturing the log messages from easylogging++
 * and forwarding it to rust's tracing.
 */
namespace monero_rust_log
{
    // static variable to make sure we don't install twice.
    bool installed = false;
    std::string span_name;

    /**
     * A dispatch callback that forwards all log messages to Rust.
     */
    class RustDispatch final : public el::LogDispatchCallback
    {
    protected:
        void handle(const el::LogDispatchData *data) noexcept override
        {
            if (!installed)
                return;

            // Get the log message.
            auto *m = data->logMessage();

            // Convert the log level to an int for easier ffi
            // (couldn't get the damn enum to work).
            uint8_t level;
            switch (m->level())
            {
            case el::Level::Trace:
                level = 0;
                break;
            case el::Level::Debug:
                level = 0; // monero prints a LOT of debug messages, so we mark them as trace logs as well
                break;
            case el::Level::Info:
                level = 2;
                break;
            case el::Level::Warning:
                level = 3;
                break;
            case el::Level::Error:
            case el::Level::Fatal:
                level = 4;
                break;
            default:
                level = 1; // Default to debug.
                break;
            }

            // Call the rust function to forward the log message.
            forward_cpp_log(
                span_name.c_str(),
                level,
                m->file().length() > 0 ? m->file() : "",
                m->line(),
                m->func(),
                m->message());
        }
    };

    /**
     * Install a callback to the easylogging++ logging system that forwards all log
     * messages to Rust.
     */
    inline void install_log_callback(const std::string &name)
    {
        if (installed)
            return;
        installed = true;
        span_name = std::string(name);

        // Pass all log messages to the RustDispatch callback above.
        el::Helpers::installLogDispatchCallback<RustDispatch>("rust-forward");

        // Disable all existing easylogging++ log writers such that messages are **only**
        // forwarded through the RustDispatch callback above. This prevents them from
        // being printed directly to stdout/stderr or written to files.
        el::Loggers::reconfigureAllLoggers(el::ConfigurationType::ToStandardOutput, "false");
        el::Loggers::reconfigureAllLoggers(el::ConfigurationType::ToFile, "false");

        // Create a default configuration such that newly created loggers will not
        // print to stdout/stderr or files by default.
        el::Configurations defaultConf;
        defaultConf.set(el::Level::Global, el::ConfigurationType::ToStandardOutput, "false");
        defaultConf.set(el::Level::Global, el::ConfigurationType::ToFile, "false");
        el::Loggers::setDefaultConfigurations(defaultConf, true /* enable default for new loggers */);

        // Disable the PERF logger, which measures... some performance stuff:
        // 2025-05-12T23:45:19.517995Z  INFO monero_cpp: PERF      364    process_new_transaction function="tools::LoggingPerformanceTimer::~LoggingPerformanceTimer()"
        // 2025-05-12T23:45:19.518013Z  INFO monero_cpp: PERF             ---------- function="tools::LoggingPerformanceTimer::LoggingPerformanceTimer(const std::string &, const std::string &, uint64_t, el::Level)"
        el::Configurations perfConf;
        perfConf.set(el::Level::Global, el::ConfigurationType::Enabled, "false");
        el::Logger *perfLogger = el::Loggers::getLogger("PERF");
        perfLogger->configure(perfConf);
    }

    /**
     * Uninstall the log callback
     */
    inline void uninstall_log_callback()
    {
        el::Helpers::uninstallLogDispatchCallback<RustDispatch>("rust-forward");
        el::Loggers::flushAll();

        installed = false;
    }
} // namespace
