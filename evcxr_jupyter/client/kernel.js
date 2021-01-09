// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

define([
    'require',
    'base/js/namespace',
    'codemirror/lib/codemirror',
    'base/js/events',
    './lint.js'
], function (requireJs, Jupyter, CodeMirror, events) {
    "use strict";

    function initCell(cell) {
        // It could be nice to show errors and warnings in the gutter as well.
        // We can enable that with the following line, however unfortunately
        // that messes up the horizontal scroll until the user clicks in editor.
        // We can sort of fix that by delaying setting of gutters by 1 second.
        // That's too hacky though. We probably need to wait until some
        // particular thing has been initialized. Until we figure out what that
        // thing is, we leave the gutters off.

        // cell.code_mirror.setOption('gutters', ["CodeMirror-lint-markers"])
        cell.code_mirror.setOption('lint', true);
    }

    function cellCreated(event, nbcell) {
        initCell(nbcell.cell);
    }

    function initExistingCells() {
        for (let cell of Jupyter.notebook.get_cells()) {
            initCell(cell);
        }
    }

    function lintText(text) {
        return new Promise(function (resolve, reject) {
            let cargoCheckComm = Jupyter.notebook.kernel.comm_manager.new_comm('evcxr-cargo-check', {
                code: text,
            });
            cargoCheckComm.on_msg(function (msg) {
                let found = [];
                for (let problem of msg.content.data.problems) {
                    found.push({
                        from: CodeMirror.Pos(problem.start_line - 1, problem.start_column - 1),
                        to: CodeMirror.Pos(problem.end_line - 1, problem.end_column - 1),
                        severity: problem.severity,
                        message: problem.message,
                    });
                }
                resolve(found);
            });
        });
    }

    return {
        onload: function () {
            $('head').append(
                $('<link rel="stylesheet" type="text/css" />').attr('href', requireJs.toUrl('./lint.css'))
            )
            events.on('create.Cell', cellCreated);
            initExistingCells();
            CodeMirror.registerHelper("lint", "rust", lintText);
        }
    }

});
