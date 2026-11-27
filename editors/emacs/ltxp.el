;;; ltxp.el --- Instant LaTeX preview with exact two-way SyncTeX -*- lexical-binding: t -*-

;; URL: https://github.com/SichangHe/latex_live_preview
;; Version: 0.0.2
;; Package-Requires: ((emacs "29.1"))
;; SPDX-License-Identifier: UPL-1.0

;;; Commentary:
;; Runs the `ltxp' language server through Eglot for TeX buffers.  Typing updates the
;; preview in the browser; clicking the preview moves point to the clicked character; moving
;; point marks the same spot in the preview.  Call `ltxp-setup' from your init file.

;;; Code:
(require 'eglot)

(defgroup ltxp nil "Instant LaTeX preview." :group 'tex)

(defcustom ltxp-command '("ltxp")
  "Command and arguments that start the ltxp language server."
  :type '(repeat string))

(defcustom ltxp-cursor-delay 0.1
  "Seconds after point stops moving before the preview marker follows it."
  :type 'number)

(defconst ltxp-modes '(latex-mode LaTeX-mode tex-mode TeX-mode plain-tex-mode latex-ts-mode)
  "Major modes whose buffers use ltxp.")

(defvar-local ltxp--timer nil)

(defun ltxp--send-cursor (buffer)
  "Tell the server where point is in BUFFER."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when-let ((server (eglot-current-server)))
        (jsonrpc-notify server :ltxp/cursor
                        (list :textDocument (eglot--TextDocumentIdentifier)
                              :position (eglot--pos-to-lsp-position)))))))

(defun ltxp--post-command ()
  "Send the cursor position once point rests."
  (when ltxp--timer (cancel-timer ltxp--timer))
  (setq ltxp--timer (run-with-timer ltxp-cursor-delay nil #'ltxp--send-cursor (current-buffer))))

;;;###autoload
(define-minor-mode ltxp-mode
  "Live LaTeX preview through the ltxp language server."
  :lighter " ltxp"
  (if ltxp-mode
      (progn (add-hook 'post-command-hook #'ltxp--post-command nil t)
             (eglot-ensure))
    (remove-hook 'post-command-hook #'ltxp--post-command t)))

;;;###autoload
(defun ltxp-setup ()
  "Use ltxp for TeX buffers."
  (add-to-list 'eglot-server-programs (cons ltxp-modes (lambda (&rest _) ltxp-command)))
  (dolist (mode ltxp-modes)
    (add-hook (intern (format "%s-hook" mode)) #'ltxp-mode)))

(provide 'ltxp)
;;; ltxp.el ends here
