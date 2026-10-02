#!/usr/bin/env python3
import importlib.machinery
import json
import os
import sqlite3
import time
import e2e
from pathlib import Path
import tempfile
import unittest

helper = importlib.machinery.SourceFileLoader('requests_helper', str(Path(__file__).resolve().parents[1]/'scripts/codex24h-requests')).load_module()

class RequestsTest(unittest.TestCase):
    def read(self, records, partial=''):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'rollout-test.jsonl'
            path.write_text(''.join(json.dumps(v)+'\n' for v in records)+partial)
            return helper.read_session(path)
    def test_event_mirror_does_not_duplicate_but_repeated_requests_survive(self):
        event={'type':'event_msg','payload':{'type':'user_message','message':'重复请求'}}
        mirror={'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':'重复请求'}]}}
        answer={'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'回答'}]}}
        tool={'type':'response_item','payload':{'type':'function_call_output','output':'RAW_TOOL_PAYLOAD'}}
        result=self.read([event,mirror,answer,tool,event,mirror,answer],'{"partial":')
        self.assertEqual([r['text'] for r in result['requests']],['重复请求']*2)
        self.assertNotIn('entries',result)
        self.assertNotIn('回答',json.dumps(result,ensure_ascii=False))
        self.assertNotIn('RAW_TOOL_PAYLOAD',json.dumps(result))
    def test_response_only_schema_images_and_environment(self):
        def user(text):return {'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_text','text':text}]}}
        picture={'type':'response_item','payload':{'type':'message','role':'user','content':[{'type':'input_image','image_url':'data:not-rendered'}]}}
        result=self.read([user('<environment_context>runtime</environment_context>'),user('first'),picture,user('first')])
        self.assertEqual([r['text'] for r in result['requests']],['first','','first'])
    def test_owned_rollout_does_not_pick_newer_other_session(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp)
            ours=home/'rollout-ours.jsonl';ours.touch()
            (home/'rollout-newer.jsonl').touch()
            with ours.open():
                self.assertEqual(helper.rollout(os.getpid(),home),ours)
    def test_completed_corrupt_record_reports_error(self):
        with self.assertRaises(json.JSONDecodeError):self.read([], 'bad record\n')
    def test_shared_history_is_scoped_ordered_and_read_only(self):
        session='00000000-0000-4000-8000-000000000001'
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);path=home/'thread_history_1.sqlite'
            with sqlite3.connect(path) as db:
                db.execute('create table thread_items(thread_id,item_type,rollout_ordinal,item_json)')
                db.executemany('insert into thread_items values(?,?,?,?)',[
                    (session,'userMessage',2,json.dumps({'content':[{'type':'text','text':'second'}]})),
                    ('other','userMessage',1,json.dumps({'content':[{'type':'text','text':'other'}]})),
                    (session,'agentMessage',3,json.dumps({'content':[{'type':'text','text':'assistant'}]})),
                    (session,'userMessage',1,json.dumps({'content':[{'type':'text','text':'first'}]}))])
            before=path.read_bytes()
            result=helper.read_shared_session(home,session)
            self.assertEqual([r['text'] for r in result['requests']],['first','second'])
            self.assertEqual(path.read_bytes(),before)
            with self.assertRaises(ValueError):helper.read_shared_session(home,'not-a-uuid')
if __name__=='__main__':unittest.main()
