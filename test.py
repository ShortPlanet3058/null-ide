def test_long_python_function_without_imports():
    # This is a long test function with no imports
    # It performs various operations using only built-in Python functionality
    
    # Test data creation
    numbers = list(range(100))
    strings = [f"item_{i}" for i in range(50)]
    mixed_data = [{"id": i, "value": i * 2, "label": f"test_{i}"} for i in range(25)]
    
    # Mathematical operations
    total_sum = sum(numbers)
    even_numbers = [n for n in numbers if n % 2 == 0]
    odd_numbers = [n for n in numbers if n % 2 == 1]
    squared = [n * n for n in numbers[:10]]
    
    # String operations
    upper_strings = [s.upper() for s in strings]
    joined_string = "-".join(strings[:5])
    split_result = joined_string.split("-")
    stripped = [s.strip() for s in ["  hello  ", "  world  ", "  test  "]]
    
    # Dictionary operations
    dict_from_lists = dict(zip(range(5), ["a", "b", "c", "d", "e"]))
    filtered_dict = {k: v for k, v in dict_from_lists.items() if k % 2 == 0}
    dict_values = list(filtered_dict.values())
    dict_keys = list(filtered_dict.keys())
    
    # Control flow
    count_even = 0
    count_odd = 0
    for n in numbers:
        if n % 2 == 0:
            count_even += 1
        else:
            count_odd += 1
    
    # Nested loops
    matrix = []
    for i in range(5):
        row = []
        for j in range(5):
            row.append(i * j)
        matrix.append(row)
    
    # More complex operations
    flattened = [item for sublist in matrix for item in sublist]
    unique_values = list(set(flattened))
    sorted_values = sorted(unique_values)
    reversed_values = list(reversed(sorted_values))
    
    # Conditional expressions
    result = [x if x % 3 == 0 else x + 1 for x in range(20)]
    
    # All operations use only built-in Python features
    # No external imports are used
    assert len(numbers) == 100
    assert len(strings) == 50
    assert len(mixed_data) == 25
    assert total_sum == 4950
    assert len(even_numbers) == 50
    assert len(odd_numbers) == 50
    assert squared == [0, 1, 4, 9, 16, 25, 36, 49, 64, 81]
    assert upper_strings[0] == "ITEM_0"
    assert joined_string == "item_0-item_1-item_2-item_3-item_4"
    assert split_result == ["item_0", "item_1", "item_2", "item_3", "item_4"]
    assert stripped == ["hello", "world", "test"]
    assert dict_from_lists == {0: "a", 1: "b", 2: "c", 3: "d", 4: "e"}
    assert filtered_dict == {0: "a", 2: "c", 4: "e"}
    assert dict_values == ["a", "c", "e"]
    assert dict_keys == [0, 2, 4]
    assert count_even == 50
    assert count_odd == 50
    assert matrix[2][3] == 6  # 2 * 3
    assert len(flattened) == 25
    assert len(unique_values) <= 25
    assert sorted_values == sorted(unique_values)
    assert reversed_values == list(reversed(sorted_values))
    assert result[0] == 0
    assert result[1] == 2  # 1 + 1
    assert result[3] == 3
    assert result[4] == 5  # 4 + 1
    assert result[6] == 6
    assert result[7] == 8  # 7 + 1
    assert result[9] == 9
    assert result[10] == 11  # 10 + 1
    assert result[12] == 12
    assert result[13] == 14  # 13 + 1
    assert result[15] == 15
    assert result[16] == 17  # 16 + 1
    assert result[18] == 18
    assert result[19] == 20  # 19 + 1
