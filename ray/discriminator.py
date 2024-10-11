import hashlib
import base58
from solana.rpc.api import Client, Signature


# Define the Solana RPC endpoint (you can use a public RPC endpoint or your own node)
rpc_url = "https://neat-snowy-bird.solana-mainnet.quiknode.pro/14c0721161ba1af1c4ef91b0a568e2b24edeb9c5/"  # Use the appropriate cluster (e.g., testnet or devnet)
client = Client(rpc_url)


def hex_to_base58(hex_string):
    # Remove '0x' prefix if present
    hex_string = hex_string.lower().replace('0x', '')
    
    # Convert hex string to bytes
    byte_data = bytes.fromhex(hex_string)
    
    # Encode bytes to Base58
    base58_string = base58.b58encode(byte_data).decode('utf-8')
    
    return base58_string

def calculate_account_discriminator(struct_name):
    # Calculate SHA-256 hash of the struct name
    hash_object = hashlib.sha256(struct_name.encode('utf-8'))
    # Get the first 8 bytes of the hash
    discriminator = hash_object.digest()[:8]
    return discriminator.hex()

# Example usage
# struct_name = 'account:PoolState'
# discriminator = calculate_account_discriminator(struct_name)
# print(hex_to_base58(discriminator))


# Transaction signature (replace this with your specific transaction signature)
transaction_signature = Signature.from_string("3S3Vi7miE7vhxqVTer6gVqAyU219DQGmEE9zz9df8TjSoVQjYDWkRzcoKXr6Hm6UBrURT9BTqpTiw5zv7dZLLN9Y")

# Fetch the transaction details
try:
    response = client.get_transaction(transaction_signature)

    # Check if the request was successful
    if response.get("result"):
        transaction_info = response["result"]
        print("Transaction details successfully retrieved.")
    else:
        print("Error retrieving transaction details:", response.get("error"))
except Exception as e:
    print("An error occurred while fetching the transaction:", str(e))

# # Dump the transaction details to a file (optional)
# import json

# with open("transaction_details.json", "w") as outfile:
#     json.dump(transaction_info, outfile, indent=4)

# print("Transaction details saved to 'transaction_details.json'.")
