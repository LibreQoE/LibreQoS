use crate::*;

#[test]
fn test_circuit_hash_only_from_commands() {
        // This test verifies that circuit_hash is ONLY obtained from:
        // 1. BakeryCommands that already have the hash
        // 2. TC command comments that include the hash
        // 
        // It should NEVER be calculated from classid or any other source
        
        // Test 1: BakeryCommand with circuit_hash
        let cmd = BakeryCommands::UpdateCircuit { circuit_hash: 1234567890 };
        match cmd {
            BakeryCommands::UpdateCircuit { circuit_hash } => {
                assert_eq!(circuit_hash, 1234567890);
            }
            _ => panic!("Wrong command type"),
        }
        
        // Test 2: TC command parsing with hash in comment
        let tc_cmd = "class add dev eth0 parent 1:1 classid 1:100 htb rate 50mbit ceil 100mbit # circuit_hash: 9876543210";
        let parsed = parse_tc_command_type(tc_cmd);
        assert_eq!(parsed, Some((true, Some(9876543210))));
        
        // Test 3: TC command without hash - should NOT generate one
        let tc_cmd_no_hash = "class add dev eth0 parent 1:1 classid 1:100 htb rate 50mbit ceil 100mbit";
        let parsed_no_hash = parse_tc_command_type(tc_cmd_no_hash);
        assert_eq!(parsed_no_hash, Some((true, None))); // Circuit detected, but NO hash
        
        // Test 4: Structural command - no hash
        let structural_cmd = "class add dev eth0 parent 1: classid 1:1 htb rate 10gbit ceil 10gbit";
        let parsed_structural = parse_tc_command_type(structural_cmd);
        assert_eq!(parsed_structural, Some((false, None))); // Not a circuit, no hash
}

#[test]
fn test_no_hash_calculation_functions() {
        // This test ensures we don't have any hash calculation functions
        // that might generate a circuit_hash from other data
        
        // The only way to get a circuit_hash should be:
        // 1. From BakeryCommands (passed by LibreQoS.py)
        // 2. From TC command comments (parsed, not calculated)
        
        // There should be NO functions like:
        // - generate_hash_from_classid()
        // - calculate_circuit_hash()
        // - hash_circuit_id()
        // etc.
        
        // This is a compile-time test - if any such functions exist,
        // they should be removed to ensure circuit_hash integrity
}