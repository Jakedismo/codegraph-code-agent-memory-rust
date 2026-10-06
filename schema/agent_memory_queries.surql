-- Candidate streams already enforce authority, lifecycle, applicability and embedding identity.
-- All rows use the logical claim ID, after collapsing vector chunks to one rank per claim.
DEFINE FUNCTION OVERWRITE fn::memory_fuse($lists: array) {
    LET $active = array::filter($lists, |$list| array::len($list) > 0);
    IF array::len($active) = 0 { RETURN []; };
    -- Keep the complete bounded union before deterministic tie-breaking/truncation.
    LET $fused = search::rrf($active, 400, 60);
    RETURN SELECT id, rrf_score / array::len($active) AS score
        FROM $fused ORDER BY score DESC, id ASC LIMIT 100;
} PERMISSIONS FULL;
