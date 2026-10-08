import unittest
from test_ipa_device import scoped_outcome, power_evidence


class CollectorTests(unittest.TestCase):
    package = 'org.touchhle.android.a64test'
    marker = 'playcover-test-unique'

    def test_power_evidence_contains_only_explicit_safe_fields(self):
        result = power_evidence('15\n', '  mStayOn=true\n  mWakefulness=Awake\n  mIsPowered=true\n  mPlugType=2\nsecretAccount=hidden\n')
        self.assertEqual(result, {'stay_on_while_plugged_in': 15, 'mStayOn': 'true',
                                 'mWakefulness': 'Awake', 'mIsPowered': 'true', 'mPlugType': '2'})
        self.assertIsNone(power_evidence('null', 'mWakefulness=unexpected')['mWakefulness'])

    def test_old_brief_panic_excluded_current_arm64_error_retained(self):
        log = '\n'.join([
            'I/SDL/APP (32480): Panic at src/objc/messages.rs:38: old run',
            'I/PlayCoverTest (100): ' + self.marker,
            'I/ActivityManager (101): Start proc 32559:' + self.package + ':game/u0a10 for activity',
            'I/SDL/APP (32480): Panic at src/objc/messages.rs:38: delayed old run',
            'I/SDL/APP (32559): touchHLE errored: missing shared-cache range lr=0x1800c3e24',
        ])
        result = scoped_outcome(log, self.marker, self.package)
        self.assertEqual(result['run_pid'], 32559)
        self.assertIsNone(result['first_panic'])
        self.assertIn('0x1800c3e24', result['runtime_error'])
        self.assertIsNone(result['outcome_scope_error'])

    def test_threadtime_pid_uses_observed_launch_identity(self):
        log = '\n'.join([
            '10-07 12:10:00.111 100 100 I PlayCoverTest: ' + self.marker,
            '10-07 12:10:00.112 500 501 I SDL/APP: Panic at current.rs:2: real panic',
            '10-07 12:10:00.113 900 901 E AndroidRuntime: Panic at other.rs:9: another app',
        ])
        result = scoped_outcome(log, self.marker, self.package, [500])
        self.assertEqual(result['first_panic'], 'Panic at current.rs:2: real panic')
        self.assertEqual(result['run_pid'], 500)

    def test_actual_game_suffix_excludes_claude_and_other_package_processes(self):
        log = '\n'.join([
            '10-07 20:20:52.585 1026 1026 I PlayCoverTest: ' + self.marker,
            '10-07 20:20:52.695 1802 1869 I ActivityManager: Start proc 1042:org.touchhle.android.a64test:game/u0a492 for next-top-activity',
            '10-07 20:21:06.609 1042 1150 I SDL/APP: touchHLE errored: missing range lr=0x1800c3e24',
            '10-07 20:21:12.561 1802 1869 I ActivityManager: Start proc 1217:org.touchhle.android:game/u0a494 for next-top-activity',
            '10-07 20:21:12.600 1217 1250 I SDL/APP: Panic at claude32.rs:1: unrelated',
            '10-07 20:21:13.100 1802 1869 I ActivityManager: Start proc 1400:org.touchhle.android.a64test/u0a492 for activity',
        ])
        result = scoped_outcome(log, self.marker, self.package)
        self.assertEqual(result['run_pid'], 1042)
        self.assertIsNone(result['first_panic'])
        self.assertIn('0x1800c3e24', result['runtime_error'])

    def test_missing_marker_pid_or_restarted_process_is_not_success(self):
        line = 'I/PlayCoverTest (100): ' + self.marker
        for log, pids in [('I/SDL/APP (5): Panic at old', [5]),
                          (line, []), (line, [5, 6]), (line + '\n' + line, [5])]:
            with self.subTest(log=log, pids=pids):
                result = scoped_outcome(log, self.marker, self.package, pids)
                self.assertIsNotNone(result['outcome_scope_error'])
                self.assertIsNone(result['first_panic'])
                self.assertIsNone(result['runtime_error'])


if __name__ == '__main__':
    unittest.main()
