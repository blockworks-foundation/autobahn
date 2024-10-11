import os

def add_my_to_names_in_file(file_path, names_to_modify):
    # Read the file
    with open(file_path, 'r') as file:
        content = file.read()
    
    # Replace each name with 'My' + name
    for name in names_to_modify:
        content = content.replace(name, f'My{name}')
    
    # Write the updated content back to the same file
    with open(file_path, 'w') as file:
        file.write(content)

def process_folder(folder_path, names_to_modify):
    # Iterate through all files in the folder
    for filename in os.listdir(folder_path):
        file_path = os.path.join(folder_path, filename)
        
        # Check if the path is a file
        if os.path.isfile(file_path):
            add_my_to_names_in_file(file_path, names_to_modify)

if __name__ == "__main__":
    # List of names that should have 'My' prepended
    names_to_modify = [
        "EncodedConfirmedTransactionWithStatusMeta",
        "EncodedTransactionWithStatusMeta",
        "UiTransactionStatusMeta",
        "TransactionError",
        "TransactionResult",
        "UiInnerInstructions",
        "UiInstruction",
        "UiCompiledInstruction",
        "UiTransactionTokenBalance",
        "UiTokenAmount",
        "Reward",
        "Rewards",
        "RewardType",
        "UiLoadedAddresses",
        "UiTransactionReturnData",
        "UiReturnDataEncoding",
        "UiTransaction",
        "UiParsedMessage",
        "ParsedAccount",
        "UiParsedInstruction",
        "ParsedInstruction",
        "UiPartiallyDecodedInstruction",
        "UiAddressTableLookup"
    ]

    # Specify the folder path here
    folder_path = '/Users/jackfisher/Desktop/new-audits/raydium-cp-swap/fomo3d-raydium-cp-swap-client/cp-swap-txs'

    # Call the function to process the folder
    process_folder(folder_path, names_to_modify)

    print("Process completed!")
