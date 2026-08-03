/**
 * The wording that separates osstat's two verification tiers.
 *
 * **This label is the feature.** A pinned model is checked against a hash
 * reviewed in a pull request against this repository, so the bytes are the ones
 * somebody looked at. A searched one is checked against the hash Hugging Face
 * reports beside the file — which proves the transfer was not corrupted and
 * cannot prove the upload was not replaced, because the digest and the file
 * come from the same origin.
 *
 * A searched model that appeared exactly like a pinned one, with no visible
 * difference, would quietly retire a guarantee SECURITY.md still makes. So the
 * words live in one constant rather than being retyped somewhere they could be
 * dropped — and the constant lives here, outside either page, because the two
 * tiers now have to stay distinguishable in two places: the LLM tab's model
 * list, and the chat header's model dropdown. A model that reached the dropdown
 * unmarked would be the same file described two ways by the same application.
 */
export const UNREVIEWED = 'Not reviewed · hash from Hugging Face';
