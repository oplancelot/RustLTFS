# UID Fix Test Script - Comprehensive Validation

## 📋 Test Overview

**Objective**: Verify that the UID allocation fix prevents duplicate UID conflicts when writing files to nested directories.

**Issue**: Files and directories were receiving the same UID, causing "Duplicate UID" errors when reading the index.

**Fix**: Deferred file UID allocation until after directory creation completes.

---

## 🧪 Test Environment

```
Tape Drive: TAPE1 (LTO-7)
Test Files:
  - test1.exe (15247741 bytes)
  - test2.exe (15247741 bytes)
  - test3.exe (copy of test1.exe)
RustLTFS Version: v0.1.0 (UID_FIX_COMPLETE)
```

---

## ⚠️ Pre-Test: Format Tape (Optional)

**WARNING**: This will ERASE all data on TAPE1!

```powershell
# Only run if you need a clean tape
rustltfs.exe format --tape TAPE1 --force
```

Expected Output:
```
✅ Tape formatted successfully
Volume UUID: <new-uuid>
```

---

## 🧪 Test Case 1: Baseline - File to Root Directory

### Purpose
Verify basic file write to root directory (no directory creation needed).

### Steps

```powershell
# Write first file to root
rustltfs.exe write test1.exe / --tape TAPE1
```

### Expected Results

```
✅ Write Operation Completed
  Files written: 1
  Bytes written: 14.54 MB
```

### Verification

```powershell
# Read index back
rustltfs.exe read --tape TAPE1
```

Expected Output:
```
📊 Tape Index Information:
  • Total Files: 1

LTFS Directory Tree:
📄 test1.exe (15247741 bytes)

✅ SUCCESS: No UID conflicts
```

### UID Check
- Root Directory: UID=1
- test1.exe: UID=2

---

## 🧪 Test Case 2: Critical - File to New Directory

### Purpose
**This is the bug reproduction test!** Writing to a new directory previously caused UID conflict.

### Steps

```powershell
# Write file to new directory /test
rustltfs.exe write test2.exe /test --tape TAPE1
```

### Expected Results - After Fix

```
✅ Write Operation Completed
  Files written: 1
  Bytes written: 14.54 MB

Logs should show:
[INFO] Creating new directory: 'test'
[INFO] New directory UID: 4
[INFO] Allocated UID 5 for file 'test2.exe' after directory creation
```

### Verification

```powershell
# Read index back - THIS MUST SUCCEED
rustltfs.exe read --tape TAPE1
```

Expected Output:
```
📊 Tape Index Information:
  • Total Files: 2

LTFS Directory Tree:
📄 test1.exe (15247741 bytes)
📁 test/
  └─ 📄 test2.exe (15247741 bytes)

✅ SUCCESS: No duplicate UID errors
```

### UID Allocation Check - Critical Validation

Expected UID sequence:
- Root Directory: UID=1
- test1.exe: UID=2
- /test directory: UID=4 (UID=3 is previous index)
- test2.exe: UID=5 ⭐ **NOT UID=4** (this was the bug!)

### Before Fix (Bug Behavior)
```
❌ ERROR: Duplicate UID 4 found in file 'test2.exe'
Parse error: Duplicate UID 4 found in file 'test2.exe'
```

### After Fix (Expected Behavior)
```
✅ Successfully parsed LTFS index
✅ No UID conflicts
✅ All files readable
```

---

## 🧪 Test Case 3: Deep Nesting - Multiple Directory Levels

### Purpose
Verify UID allocation works correctly with deeply nested directory structures.

### Steps

```powershell
# Create nested directory structure: /a/b/c/test3.exe
rustltfs.exe write test3.exe /a/b/c --tape TAPE1
```

### Expected Results

```
✅ Write Operation Completed

Logs should show sequential UID allocation:
[INFO] Creating new directory: 'a'
[INFO] New directory UID: 6
[INFO] Creating new directory: 'b'
[INFO] New directory UID: 7
[INFO] Creating new directory: 'c'
[INFO] New directory UID: 8
[INFO] Allocated UID 9 for file 'test3.exe' after directory creation
```

### Verification

```powershell
rustltfs.exe read --tape TAPE1
```

Expected Output:
```
📊 Tape Index Information:
  • Total Files: 3

LTFS Directory Tree:
📄 test1.exe (15247741 bytes)
📁 test/
  └─ 📄 test2.exe (15247741 bytes)
📁 a/
  └─ 📁 b/
      └─ 📁 c/
          └─ 📄 test3.exe (15247741 bytes)

✅ SUCCESS: All nested directories created correctly
```

### UID Allocation Check

Expected sequence:
- UID=1: Root directory
- UID=2: test1.exe
- UID=3: (Used by index write)
- UID=4: /test directory
- UID=5: test2.exe
- UID=6: /a directory
- UID=7: /a/b directory
- UID=8: /a/b/c directory
- UID=9: test3.exe ✅

No duplicate UIDs should exist!

---

## 🧪 Test Case 4: Index Export and Validation

### Purpose
Export and manually inspect the LTFS index XML to verify UID uniqueness.

### Steps

```powershell
# The read command auto-saves index
rustltfs.exe read --tape TAPE1
```

### Find Saved Index

Look for file: `LTFSIndex_Load_YYYYMMDD_HHMMSS.schema`

### Manual Validation

Open the `.schema` file and check:

```xml
<ltfsindex version="2.4.0">
  <directory>
    <name></name>
    <uid>1</uid>  <!-- Root -->
    <contents>
      <file>
        <name>test1.exe</name>
        <uid>2</uid>  <!-- ✅ Unique -->
      </file>
      <directory>
        <name>test</name>
        <uid>4</uid>  <!-- ✅ Unique -->
        <contents>
          <file>
            <name>test2.exe</name>
            <uid>5</uid>  <!-- ✅ Unique - NOT 4! -->
          </file>
        </contents>
      </directory>
      <directory>
        <name>a</name>
        <uid>6</uid>  <!-- ✅ Unique -->
        <contents>
          <directory>
            <name>b</name>
            <uid>7</uid>  <!-- ✅ Unique -->
            <contents>
              <directory>
                <name>c</name>
                <uid>8</uid>  <!-- ✅ Unique -->
                <contents>
                  <file>
                    <name>test3.exe</name>
                    <uid>9</uid>  <!-- ✅ Unique -->
                  </file>
                </contents>
              </directory>
            </contents>
          </directory>
        </contents>
      </directory>
    </contents>
  </directory>
  <highestfileuid>9</highestfileuid>  <!-- ✅ Correct -->
</ltfsindex>
```

### Validation Criteria

- [x] All UIDs are unique
- [x] No UID appears twice
- [x] UIDs are sequential (gaps allowed for index writes)
- [x] highestfileuid matches the last allocated UID
- [x] Directories and files have different UIDs

---

## 🧪 Test Case 5: LTFSCopyGUI Cross-Compatibility

### Purpose
Verify that LTFSCopyGUI can read RustLTFS-written tapes.

### Steps

1. Use LTFSCopyGUI to open TAPE1
2. Browse directory structure
3. Verify all files are visible
4. Try to read a file

### Expected Results

```
✅ LTFSCopyGUI shows correct directory tree:
   - test1.exe (root)
   - test/test2.exe
   - a/b/c/test3.exe

✅ All files are accessible
✅ No errors or warnings
✅ File sizes match
```

---

## 🧪 Test Case 6: Stress Test - Multiple Files and Directories

### Purpose
Test UID allocation under more realistic workload.

### Steps

```powershell
# Write multiple files to various directories
rustltfs.exe write file1.txt /dir1 --tape TAPE1
rustltfs.exe write file2.txt /dir1 --tape TAPE1
rustltfs.exe write file3.txt /dir2 --tape TAPE1
rustltfs.exe write file4.txt /dir2/subdir --tape TAPE1
```

### Expected Results

All writes succeed, and:
```
rustltfs.exe read --tape TAPE1

📊 Total Files: 7
📁 dir1/
  ├─ 📄 file1.txt
  └─ 📄 file2.txt
📁 dir2/
  ├─ 📄 file3.txt
  └─ 📁 subdir/
      └─ 📄 file4.txt

✅ No UID conflicts
✅ All files readable
```

---

## ✅ Success Criteria

### Must Pass (Critical)

- [x] Test Case 2 completes without "Duplicate UID" error
- [x] All index reads succeed
- [x] UIDs are unique across all files and directories
- [x] highestfileuid is correctly maintained

### Should Pass (Important)

- [x] Deep nesting works correctly (Test Case 3)
- [x] LTFSCopyGUI can read the tape
- [x] Index XML validates correctly
- [x] No errors in log files

### Nice to Have

- [ ] Performance metrics acceptable
- [ ] Memory usage reasonable
- [ ] Log messages clear and helpful

---

## 🐛 Regression Check - Bug Reproduction

### If Fix Reverted (Negative Test)

To verify the fix is working, you can temporarily revert and see the bug:

```rust
// TEMPORARY - DO NOT COMMIT
let new_uid = current_index.highestfileuid.unwrap_or(0) + 1;
let new_file = File { uid: new_uid, ... }; // Allocate BEFORE directory creation
```

Expected Failure:
```
❌ Duplicate UID 4 found in file 'test2.exe'
❌ Index parsing failed
```

---

## 📊 Test Results Summary

### Test Execution Log

| Test Case | Status | UID Conflict | Notes |
|-----------|--------|--------------|-------|
| 1. Root File | ⏳ Pending | N/A | Baseline test |
| 2. New Directory | ⏳ Pending | Must Fix! | Critical test |
| 3. Deep Nesting | ⏳ Pending | No | Stress test |
| 4. Index Export | ⏳ Pending | No | Manual validation |
| 5. LTFSCopyGUI | ⏳ Pending | No | Cross-compat |
| 6. Stress Test | ⏳ Pending | No | Production-like |

### After Testing - Update This Section

```
Test Date: [DATE]
Tester: [NAME]
Result: ✅ PASS / ❌ FAIL

Summary:
- All critical tests passed
- No UID conflicts detected
- LTFSCopyGUI compatibility confirmed
- Production ready: YES/NO

Issues Found:
[List any issues]

Recommendations:
[List any recommendations]
```

---

## 🔍 Debug Commands

### If Tests Fail

```powershell
# Enable verbose logging
$env:RUST_LOG="info,rust_ltfs=debug"
rustltfs.exe read --tape TAPE1

# Check tape position
rustltfs.exe status --tape TAPE1

# Export index for manual inspection
# (automatically done during read command)
```

### Log Analysis

Look for these patterns in logs:
```
✅ Good: "Allocated UID X for file 'Y' after directory creation"
✅ Good: "New directory UID: X"
❌ Bad: "Duplicate UID X found"
❌ Bad: "XML parsing failed"
```

---

## 📝 Notes

1. **First Run**: May take longer due to tape initialization
2. **Index Updates**: Each write updates the index on tape
3. **UID Gaps**: Normal - index writes consume UIDs
4. **Performance**: Sequential UID allocation is very fast
5. **Rollback**: Keep backup of previous version if needed

---

**Status**: ⏳ READY FOR EXECUTION
**Priority**: 🔴 P0 - CRITICAL
**Estimated Time**: 15-20 minutes
**Required**: Physical LTO-7 tape drive and TAPE1
