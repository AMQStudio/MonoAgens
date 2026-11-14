"""The plumbing every body shares: splitting one act's items and cutting one text (§2.4.4)."""

from __future__ import annotations

from monoagens.adapters.support import cut, split


def test_split_batches_items_by_what_one_message_takes():
    assert split(["a", "b", "c"], 2) == [["a", "b"], ["c"]]


def test_split_never_sends_beyond_the_limit_a_body_declares():
    assert split(["a", "b", "c"], 1) == [["a"], ["b"], ["c"]]
    assert split(["a", "b", "c"], 0) == [["a"], ["b"], ["c"]]


def test_split_answers_one_empty_batch_for_an_empty_act():
    assert split([], 3) == [[]]


def test_cut_keeps_a_text_that_fits_and_cuts_one_that_does_not():
    assert cut("halo", 10) == "halo"
    assert cut("halo dunia", 4) == "halo"
    assert cut("halo", 0) == "halo"  # no limit declared: never cut
