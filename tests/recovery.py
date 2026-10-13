#!/usr/bin/env python3
"""Recovery state-machine tests; no model requests or external SMTP."""
import datetime
import json
from pathlib import Path
import runpy
import sqlite3
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import mail as fixtures

module = runpy.run_path(str(fixtures.ROOT/'scripts/codex24h-recovery'))
Recovery = module['Recovery']


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.case = fixtures.MailTests('test_capacity_mail_is_not_labelled_completed_or_goal')
        self.case.setUp()
        self.addCleanup(self.case.doCleanups)
        self.cfg,self.home = self.case.cfg,self.case.home
        self.directory = self.case.base/'bridge'; self.directory.mkdir()
        self.args = SimpleNamespace(config=self.case.config,codex_home=self.home,session='session-1',standalone=False)
        self.observer = SimpleNamespace(sessions={'session-1'},since=0)
        self.clock = 100.0
        self.addCleanup(patch.stopall)
        patch('time.monotonic',side_effect=lambda:self.clock).start()
        self.sender = patch.object(fixtures.mail,'spawn_worker').start()
        self.frontend()
        with sqlite3.connect(self.home/'thread_history_1.sqlite') as conn:
            conn.execute('CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,completed_at INTEGER)')
            conn.execute('CREATE TABLE thread_items(thread_id TEXT,item_type TEXT,item_json TEXT)')
        self.controller = self.new_controller()
        self.controller.tick()

    def new_controller(self):
        return Recovery(fixtures.mail.__dict__,self.cfg,self.args,self.directory,self.observer)

    def frontend(self,generation=0,ready=True):
        module['atomic_json'](self.directory/'frontend.json',dict(generation=generation,input_at_ms=0,ready=ready))

    def goal(self,status,id='goal-1'):
        with sqlite3.connect(self.home/'goals_1.sqlite') as conn:
            conn.execute('DELETE FROM thread_goals')
            conn.execute('INSERT INTO thread_goals VALUES (?,?,?,0)',('session-1',id,status))

    def fail(self,turn='failed-1'):
        with sqlite3.connect(self.home/'thread_history_1.sqlite') as conn:
            conn.execute('INSERT INTO thread_turns VALUES (?,?,?,0)',('session-1',turn,'failed'))
        event = {'type':'model_capacity','thread-id':'session-1','turn-id':turn}
        self.controller.capacity(event)
        return next(j for j in self.controller.pending.values() if j['turn']==turn),event

    def advance(self):
        self.clock += 11
        self.controller.tick()

    def payload(self,key):
        with fixtures.mail.database(self.cfg) as conn:
            raw,state = conn.execute('SELECT payload,state FROM jobs WHERE key=?',(key,)).fetchone()
        return json.loads(raw),state

    def attempts(self):
        with fixtures.mail.database(self.cfg) as conn:
            row=conn.execute('SELECT attempts FROM capacity_budgets WHERE session=?',('session-1',)).fetchone()
        return row[0] if row else 0

    def confirm(self,job):
        module['atomic_json'](self.directory/'recovery-ack.json',dict(key=job['key'],status='written'))
        if job['command']=='go on':
            message='auto-'+job['turn']
            self.controller.record({'dir':'from_tui','kind':'op','ts':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'payload':{'UserTurn':{'client_user_message_id':message,'items':[{'type':'text','text':'go on'}]}}})
            with sqlite3.connect(self.home/'thread_history_1.sqlite') as conn:
                conn.execute('INSERT INTO thread_items VALUES (?,?,?)',('session-1','userMessage',json.dumps({'clientId':message})))
        else:
            self.goal('active')
        self.controller.tick()

    def test_three_submissions_then_capacity_check_and_manual_reset(self):
        for n in range(1,4):
            job,event=self.fail('failure-'+str(n));self.advance()
            self.assertEqual(job['command'],'go on')
            self.assertEqual(self.payload(job['key'])[1],'recovering')
            self.assertEqual(self.sender.call_count,n-1,'no email before native confirmation')
            self.confirm(job)
            payload,state=self.payload(job['key'])
            self.assertEqual(payload['recovery'],'submitted')
            self.assertEqual(payload['recovery_attempt'],n)
            self.assertEqual(state,'pending')
            self.assertEqual(self.attempts(),n,'automatic go on must not reset')
            self.controller.capacity(event)
            self.assertFalse(self.controller.pending,'duplicate error cannot send again')
        job,_=self.fail('failure-4');self.advance()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'exhausted')
        self.assertEqual(self.attempts(),3)
        self.frontend(generation=1)
        self.controller.record({'dir':'from_tui','kind':'op','ts':datetime.datetime.now(datetime.timezone.utc).isoformat(),
             'payload':{'UserTurn':{'client_user_message_id':'manual-request','items':[{'text':'new task'}]}}})
        with sqlite3.connect(self.home/'thread_history_1.sqlite') as conn:
            conn.execute('INSERT INTO thread_items VALUES (?,?,?)',('session-1','userMessage',json.dumps({'clientId':'manual-request'})))
        self.controller.tick()
        self.assertEqual(self.attempts(),0)
        job,_=self.fail('new-failure');self.advance();self.confirm(job)
        self.assertEqual(self.attempts(),1)

    def test_paused_goal_uses_native_resume_and_waits_for_acceptance(self):
        self.goal('paused');job,_=self.fail();self.advance()
        self.assertEqual(job['command'],'/goal resume')
        module['atomic_json'](self.directory/'recovery-ack.json',dict(key=job['key'],status='written'))
        self.controller.tick()
        self.assertFalse(self.sender.called,'PTY write alone does not confirm Goal acceptance')
        self.confirm(job)
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'submitted')

    def test_goal_can_transition_from_active_to_paused_during_delay(self):
        self.goal('active');job,_=self.fail();self.goal('paused');self.advance()
        self.assertEqual(job['command'],'/goal resume')

    def test_completed_and_other_goal_states_never_receive_go_on(self):
        for n,status in enumerate(('complete','active','blocked','usage_limited','budget_limited')):
            self.goal(status);job,_=self.fail('goal-'+str(n));self.advance()
            self.assertNotIn('command',job)
            self.assertIn(self.payload(job['key'])[0]['recovery'],('blocked','skipped'))
        self.assertEqual(self.attempts(),0)

    def test_paused_goal_completed_during_jitter_and_replaced_goal_are_not_revived(self):
        self.goal('paused');job,_=self.fail();self.goal('complete');self.advance()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'skipped')
        self.goal('paused');job,_=self.fail('replacement');self.goal('paused','goal-2');self.advance()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'blocked')

    def test_missing_goal_database_is_unknown_not_an_ordinary_task(self):
        (self.home/'goals_1.sqlite').unlink()
        job,_=self.fail();self.advance()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'blocked')
        self.assertFalse((self.directory/'recovery-request.json').exists())

    def test_new_input_draft_or_session_switch_cancels_recovery(self):
        for n in range(3):
            self.frontend();self.controller.active='session-1';self.controller.proof=0
            job,_=self.fail('unsafe-'+str(n))
            if n==0:self.frontend(generation=1)
            elif n==1:self.frontend(ready=False)
            else:self.controller.active='other-session'
            self.advance()
            self.assertEqual(self.payload(job['key'])[0]['recovery'],'blocked')
        self.assertEqual(self.attempts(),0)

    def test_successful_turn_resets_budget(self):
        job,_=self.fail();self.advance();self.confirm(job)
        self.controller.reset('session-1')
        self.assertEqual(self.attempts(),0)

    def test_jitter_is_independent_per_distinct_failure_and_not_redraw(self):
        with patch('random.SystemRandom') as rng:
            rng.return_value.uniform.side_effect=(3.125,9.875)
            a,event=self.fail('a');self.controller.capacity(event)
            b,_=self.fail('b')
            self.assertEqual(rng.return_value.uniform.call_args_list, [unittest.mock.call(3,10)]*2)
            self.assertEqual(a['due'],103.125);self.assertEqual(b['due'],109.875)

    def test_duplicate_frontends_share_one_claim_and_counter(self):
        other=self.new_controller();other.tick()
        job,event=self.fail();other.capacity(event)
        self.assertFalse(other.pending)
        self.advance();self.confirm(job)
        self.assertEqual(self.attempts(),1)

    def test_missing_confirmation_does_not_claim_success_or_repeat_command(self):
        job,_=self.fail();self.advance()
        self.clock+=16;self.controller.tick()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'blocked')
        self.assertEqual(self.attempts(),1,'uncertain submission reserves budget')

    def test_mail_worker_and_retry_never_send_a_held_job(self):
        job,_=self.fail()
        with patch.object(fixtures.mail,'deliver') as deliver:
            fixtures.mail.worker(self.args.config,job['key'])
            self.assertFalse(deliver.called)
        self.assertEqual(self.payload(job['key'])[1],'recovering')

    def test_old_buffered_message_cannot_rebind_after_manual_switch(self):
        self.frontend(generation=1)
        state=module['read_json'](self.directory/'frontend.json');state['input_at_ms']=10**15
        module['atomic_json'](self.directory/'frontend.json',state)
        self.controller.record({'dir':'from_tui','kind':'op','ts':'2026-10-13T00:00:00Z',
             'payload':{'UserTurn':{'client_user_message_id':'old-buffered','items':[{'text':'old'}]}}})
        with sqlite3.connect(self.home/'thread_history_1.sqlite') as conn:
            conn.execute('INSERT INTO thread_items VALUES (?,?,?)',('session-1','userMessage',json.dumps({'clientId':'old-buffered'})))
        self.controller.tick()
        self.assertEqual(self.controller.proof,0)
        job,_=self.fail();self.advance()
        self.assertEqual(self.payload(job['key'])[0]['recovery'],'blocked')

    def test_fresh_goal_binds_only_from_native_status_and_resets_manual_budget(self):
        self.frontend(generation=1)
        stamp='2026-10-13T00:00:00.123Z'
        state=module['read_json'](self.directory/'frontend.json')
        state['input_at_ms']=1791849600123
        module['atomic_json'](self.directory/'frontend.json',state)
        self.controller.record({'dir':'to_tui','kind':'app_event','variant':'SetThreadGoalDraft','ts':stamp})
        self.controller.tick()
        request=module['read_json'](self.directory/'recovery-request.json')
        self.assertEqual(request['command'],'/status')
        self.assertEqual(request['generation'],1)
        self.assertEqual(self.controller.proof,0,'do not bind before native identity')
        self.controller.record({'dir':'codex24h','kind':'resume','session':'session-1','generation':1})
        self.assertEqual(self.controller.proof,1)
        self.assertIsNone(self.controller.discover)

    def test_abandoned_dispatch_is_reported_without_replaying_command(self):
        job,_=self.fail();self.advance()
        with fixtures.mail.database(self.cfg) as conn:
            conn.execute('UPDATE capacity_claims SET owner=? WHERE key=?',('999999999:0:dead',job['key']))
            conn.commit()
        other=self.new_controller()
        self.assertFalse(other.pending)
        payload,state=self.payload(job['key'])
        self.assertEqual(payload['recovery'],'blocked')
        self.assertIn('不重发',payload['recovery_detail'])
        self.assertEqual(state,'pending')


if __name__=='__main__':
    unittest.main(verbosity=2)
